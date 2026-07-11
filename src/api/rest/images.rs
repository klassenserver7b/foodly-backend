use crate::{AppState, error::AppError, models::image::Image, services::storage};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            post(upload_image).layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route("/{hash}", get(get_image).delete(delete_image))
}

async fn upload_image(
    State(state): State<AppState>,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Image>), AppError> {
    if bytes.is_empty() {
        return Err(AppError::Unprocessable("Empty image body".to_string()));
    }

    let ct = infer_content_type(&bytes);
    if ct == "application/octet-stream" || ct == "image/gif" {
        return Err(AppError::Unprocessable(
            "Invalid image format. Only JPEG, PNG, and WEBP are allowed".to_string(),
        ));
    }

    let hash = storage::save_image(&state.image_storage_path, &bytes).await?;

    let image = sqlx::query_as!(
        Image,
        r#"
        INSERT INTO images (hash, name)
        VALUES ($1, NULL)
        ON CONFLICT (hash) DO UPDATE SET hash = EXCLUDED.hash
        RETURNING id, hash, name
        "#,
        hash
    )
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(image)))
}

async fn get_image(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let etag_value = format!("\"{}\"", hash);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|h| h.to_str().ok())
        == Some(&etag_value)
    {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }

    if hash.len() < 4 {
        return Err(AppError::NotFound("Image not found".to_string()));
    }

    let bytes = crate::services::storage::read_image(&state.image_storage_path, &hash)
        .await
        .map_err(|_| AppError::NotFound("Image not found".to_string()))?;

    let content_type = infer_content_type(&bytes);

    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::ETAG, HeaderValue::from_str(&etag_value).unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    Ok(response)
}

async fn delete_image(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<StatusCode, AppError> {
    let result = sqlx::query!("DELETE FROM images WHERE hash = $1", hash)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Image not found".to_string()));
    }

    let _ = crate::services::storage::delete_image(&state.image_storage_path, &hash).await;

    Ok(StatusCode::NO_CONTENT)
}

pub fn infer_content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 11 && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_upload_and_get_image(pool: PgPool) {
        let tmp = std::env::temp_dir().join("foodly_test_images_ep");
        let state = State(AppState {
            pool: pool.clone(),
            image_storage_path: tmp.clone(),
        });

        // 1. Upload
        let bytes = Bytes::from(vec![0xFF, 0xD8, 0xFF, 0x01, 0x02, 0x03]); // Fake JPEG
        let (status, Json(image)) = upload_image(state.clone(), bytes).await.unwrap();
        assert_eq!(status, StatusCode::CREATED);
        assert!(!image.hash.is_empty());

        // 2. Get without ETag
        let mut headers = HeaderMap::new();
        let get_res = get_image(state.clone(), Path(image.hash.clone()), headers.clone())
            .await
            .unwrap();
        assert_eq!(get_res.status(), StatusCode::OK);
        assert_eq!(
            get_res.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
        assert_eq!(
            get_res.headers().get(header::ETAG).unwrap(),
            &format!("\"{}\"", image.hash)
        );

        // 3. Get with ETag
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&format!("\"{}\"", image.hash)).unwrap(),
        );
        let get_res_304 = get_image(state.clone(), Path(image.hash.clone()), headers)
            .await
            .unwrap();
        assert_eq!(get_res_304.status(), StatusCode::NOT_MODIFIED);

        // 4. Delete image
        let delete_status = delete_image(state.clone(), Path(image.hash.clone()))
            .await
            .unwrap();
        assert_eq!(delete_status, StatusCode::NO_CONTENT);

        // 5. Try to get deleted image
        let get_res_deleted =
            get_image(state.clone(), Path(image.hash.clone()), HeaderMap::new()).await;
        assert!(get_res_deleted.is_err());

        // 6. Try to delete deleted image
        let delete_res_deleted = delete_image(state, Path(image.hash.clone())).await;
        assert!(delete_res_deleted.is_err());

        let _ = tokio::fs::remove_dir_all(tmp).await;
    }
}

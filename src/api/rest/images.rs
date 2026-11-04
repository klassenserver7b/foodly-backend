use crate::{AppState, error::AppError, models::image::Image, services::storage};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::schema::images;

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

    let mut conn = state.pool.get().await?;

    let image = diesel::insert_into(images::table)
        .values((
            images::hash.eq(&hash),
            images::name.eq::<Option<String>>(None),
        ))
        .on_conflict(images::hash)
        .do_update()
        .set(images::hash.eq(&hash))
        .returning((images::id, images::hash, images::name))
        .get_result::<(i64, String, Option<String>)>(&mut conn)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(Image {
            id: image.0,
            hash: image.1,
            name: image.2,
        }),
    ))
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

    let bytes = storage::read_image(&state.image_storage_path, &hash)
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
    let mut conn = state.pool.get().await?;

    let result = diesel::delete(images::table)
        .filter(images::hash.eq(&hash))
        .execute(&mut conn)
        .await?;

    if result == 0 {
        return Err(AppError::NotFound("Image not found".to_string()));
    }

    let _ = storage::delete_image(&state.image_storage_path, &hash).await;

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

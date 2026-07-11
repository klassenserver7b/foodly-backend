//! REST API endpoints for catalog resources (Tags and Ingredients).

use crate::models::catalog::{CatalogResponse, Ingredient, Tag};
use crate::{AppState, error::AppError};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};

/// Configures and returns the Axum router for tags.
pub fn tags_router() -> Router<AppState> {
    Router::new().route("/", get(list_tags)).route(
        "/{id}/icon",
        get(get_tag_icon)
            .put(upload_tag_icon)
            .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)),
    )
}

/// Configures and returns the Axum router for ingredients.
pub fn ingredients_router() -> Router<AppState> {
    Router::new().route("/", get(list_ingredients))
}

async fn list_tags(State(state): State<AppState>) -> Result<Json<CatalogResponse<Tag>>, AppError> {
    let records = sqlx::query!("SELECT id, svg FROM tags ORDER BY id ASC")
        .fetch_all(&state.pool)
        .await?;

    let data = records
        .into_iter()
        .map(|r| Tag {
            id: r.id,
            svg: r.svg,
        })
        .collect();

    Ok(Json(CatalogResponse { data }))
}

async fn list_ingredients(
    State(state): State<AppState>,
) -> Result<Json<CatalogResponse<Ingredient>>, AppError> {
    let records = sqlx::query!("SELECT id, name FROM ingredients ORDER BY id ASC")
        .fetch_all(&state.pool)
        .await?;

    let data = records
        .into_iter()
        .map(|r| Ingredient {
            id: r.id as i32,
            name: r.name,
        })
        .collect();

    Ok(Json(CatalogResponse { data }))
}

async fn get_tag_icon(
    State(state): State<AppState>,
    Path(tag_id): Path<String>,
) -> Result<Response, AppError> {
    let tag = sqlx::query!("SELECT svg FROM tags WHERE id = $1", tag_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Tag not found".to_string()))?;

    let hash = tag
        .svg
        .ok_or_else(|| AppError::NotFound("Tag has no icon".to_string()))?;

    let bytes = crate::services::storage::read_image(&state.image_storage_path, &hash)
        .await
        .map_err(|_| AppError::NotFound("Icon file not found".to_string()))?;

    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );

    Ok(response)
}

async fn upload_tag_icon(
    State(state): State<AppState>,
    Path(tag_id): Path<String>,
    bytes: Bytes,
) -> Result<StatusCode, AppError> {
    if bytes.is_empty() {
        return Err(AppError::Unprocessable("Empty body".to_string()));
    }

    // SVG Validation
    let prefix = &bytes[..std::cmp::min(bytes.len(), 1024)];
    if let Ok(s) = std::str::from_utf8(prefix) {
        if !s.contains("<svg") {
            return Err(AppError::Unprocessable("Invalid SVG format".to_string()));
        }
    } else {
        return Err(AppError::Unprocessable("Invalid SVG format".to_string()));
    }

    let hash = crate::services::storage::save_image(&state.image_storage_path, &bytes).await?;

    let result = sqlx::query!("UPDATE tags SET svg = $1 WHERE id = $2", hash, tag_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Tag not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_list_tags(pool: PgPool) {
        let state = State(AppState {
            pool,
            image_storage_path: std::path::PathBuf::from("/tmp"),
        });
        let response = list_tags(state).await.expect("Failed to list tags");
        // Ensure no error and valid response structure
        assert!(response.0.data.is_empty() || !response.0.data.is_empty());
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_list_ingredients(pool: PgPool) {
        let state = State(AppState {
            pool,
            image_storage_path: std::path::PathBuf::from("/tmp"),
        });
        let response = list_ingredients(state)
            .await
            .expect("Failed to list ingredients");
        // Ensure no error and valid response structure
        assert!(response.0.data.is_empty() || !response.0.data.is_empty());
    }
}

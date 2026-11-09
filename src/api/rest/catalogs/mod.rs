//! REST API endpoints for catalog resources (tags and ingredients).

use crate::models::catalog::{CatalogResponse, Ingredient, Tag};
use crate::services::storage;
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
use diesel::prelude::*;
use diesel::result::OptionalExtension;
use diesel_async::RunQueryDsl;

use crate::schema::{ingredients, tags};

/// Configures and returns the Axum router for tags.
pub fn tags_router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_tags))
        .route("/{id}/icon", get(get_tag_icon).put(upload_tag_icon))
}

/// Configures and returns the Axum router for ingredients.
pub fn ingredients_router() -> Router<AppState> {
    Router::new().route("/", get(list_ingredients))
}

async fn list_tags(State(state): State<AppState>) -> Result<Json<CatalogResponse<Tag>>, AppError> {
    let mut conn = state.pool.get().await?;

    let records = tags::table
        .select((tags::id, tags::svg))
        .order(tags::id.asc())
        .load::<(String, Option<String>)>(&mut conn)
        .await?;

    let data = records
        .into_iter()
        .map(|(id, svg)| Tag { id, svg })
        .collect();

    Ok(Json(CatalogResponse { data }))
}

async fn list_ingredients(
    State(state): State<AppState>,
) -> Result<Json<CatalogResponse<Ingredient>>, AppError> {
    let mut conn = state.pool.get().await?;

    let records = ingredients::table
        .select((ingredients::id, ingredients::name))
        .order(ingredients::id.asc())
        .load::<(i64, String)>(&mut conn)
        .await?;

    let data = records
        .into_iter()
        .map(|(id, name)| Ingredient { id, name })
        .collect();

    Ok(Json(CatalogResponse { data }))
}

async fn get_tag_icon(
    State(state): State<AppState>,
    Path(tag_id): Path<String>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.get().await?;

    let tag = tags::table
        .find(tag_id)
        .select(tags::svg)
        .first::<Option<String>>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("Tag not found".to_string()))?;

    let hash = tag.ok_or_else(|| AppError::NotFound("Tag has no icon".to_string()))?;
    let bytes = storage::read_image(&state.image_storage_path, &hash)
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
    if let Ok(s) = std::str::from_utf8(prefix)
        && !s.contains("<svg")
    {
        return Err(AppError::Unprocessable("Invalid SVG format".to_string()));
    }

    let mut conn = state.pool.get().await?;

    let hash = storage::save_image(&state.image_storage_path, &bytes).await?;
    let result = diesel::update(tags::table)
        .filter(tags::id.eq(&tag_id))
        .set(tags::svg.eq(&hash))
        .execute(&mut conn)
        .await?;

    if result == 0 {
        return Err(AppError::NotFound("Tag not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

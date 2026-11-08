use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;

use crate::error::AppError;
use crate::models::image::Image;
use crate::services::storage;
use crate::{AppState, RateLimitMode, SecureClientIpKeyExtractor};

use crate::schema::{images, recipe_images, recipes, users};

/// Public routes: anyone can view an image by hash (no auth required).
pub fn public_router() -> Router<AppState> {
    Router::new().route("/{hash}", get(get_image))
}

/// Protected routes: require auth. Upload records the uploader as owner;
/// delete is restricted to the owner. Rate limited per client IP
/// (unless RATE_LIMIT_MODE=disabled, e.g. in tests).
pub fn protected_router() -> Router<AppState> {
    let mode = RateLimitMode::from_env();
    if mode == RateLimitMode::Disabled {
        return Router::new()
            .route(
                "/",
                post(upload_image).layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)),
            )
            .route("/{hash}", delete(delete_image));
    }

    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(60) // 1 token per 60s = 1 upload/min sustained
            .burst_size(10) // burst of 10
            .key_extractor(SecureClientIpKeyExtractor { mode })
            .finish()
            .unwrap(),
    );
    let governor_layer = GovernorLayer::new(governor_conf);

    Router::new()
        .route(
            "/",
            post(upload_image).layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route("/{hash}", delete(delete_image))
        .route_layer(governor_layer)
}

async fn upload_image(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
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

    // Re-uploading identical bytes is idempotent: the existing row is returned
    // and its owner is kept.
    let image = diesel::insert_into(images::table)
        .values((
            images::hash.eq(&hash),
            images::name.eq::<Option<String>>(None),
            images::owner_id.eq(user_id),
        ))
        .on_conflict(images::hash)
        .do_nothing()
        .returning((images::id, images::hash, images::name))
        .get_result::<(i64, String, Option<String>)>(&mut conn)
        .await
        .optional()?;

    let (image, status_code) = match image {
        Some(image) => (image, StatusCode::CREATED),
        None => {
            // Hash already exists — fetch the existing row, keeping its owner
            let image = images::table
                .filter(images::hash.eq(&hash))
                .select((images::id, images::hash, images::name))
                .first::<(i64, String, Option<String>)>(&mut conn)
                .await?;
            (image, StatusCode::OK)
        }
    };

    Ok((
        status_code,
        Json(Image {
            id: image.0,
            hash: image.1,
            name: image.2,
        }),
    ))
}

/// A valid image hash: exactly 64 lowercase hex characters (SHA-256).
/// Rejects path traversal, mixed case, and non-hex input before the
/// value is ever used as a filesystem path component.
fn validate_hash(hash: &str) -> Result<(), AppError> {
    crate::services::storage::validate_hash(hash)
        .map_err(|_| AppError::NotFound("Image not found".to_string()))
}

async fn get_image(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Validate BEFORE any other logic so a malformed hash can never
    // reach the filesystem path construction, even via the etag path.
    validate_hash(&hash)?;

    let mut conn = state.pool.get().await?;

    // Only serve images that have a row in the `images` table.
    // This prevents serving profile pictures, tag icons, or any other
    // file that happens to be in the storage directory without a
    // corresponding `images` row.
    let image = images::table
        .filter(images::hash.eq(&hash))
        .select((images::id, images::hash, images::name))
        .first::<(i64, String, Option<String>)>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("Image not found".to_string()))?;

    let etag_value = format!("\"{}\"", image.1);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|h| h.to_str().ok())
        == Some(&etag_value)
    {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }

    let bytes = storage::read_image(&state.image_storage_path, &image.1)
        .await
        .map_err(|_| AppError::NotFound("Image not found".to_string()))?;
    let content_type = infer_content_type(&bytes);

    let mut response = bytes.into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response_headers.insert(header::ETAG, HeaderValue::from_str(&etag_value).unwrap());
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    Ok(response)
}

async fn delete_image(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(hash): Path<String>,
) -> Result<StatusCode, AppError> {
    validate_hash(&hash)?;

    let mut conn = state.pool.get().await?;

    // Find the image (must exist and be owned by the caller)
    let image = images::table
        .filter(images::hash.eq(&hash))
        .select((images::id, images::owner_id))
        .first::<(i64, Option<i64>)>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("Image not found".to_string()))?;

    let (image_id, owner_id) = image;

    if owner_id != Some(user_id) {
        return Err(AppError::NotFound("Image not found".to_string()));
    }

    // Check if anyone uses this hash as their profile picture
    let profile_pic_used: bool = diesel::select(diesel::dsl::exists(
        users::table.filter(users::profile_picture.eq(&hash)),
    ))
    .first(&mut conn)
    .await?;

    // Check if the image is referenced by any recipe (as main image or gallery image)
    let is_referenced: bool = diesel::select(diesel::dsl::exists(
        recipe_images::table.filter(recipe_images::image_id.eq(image_id)),
    ))
    .first(&mut conn)
    .await?;

    let is_main: bool = diesel::select(diesel::dsl::exists(
        recipes::table.filter(recipes::main_image_id.eq(image_id)),
    ))
    .first(&mut conn)
    .await?;

    if profile_pic_used || is_referenced || is_main {
        // Image is still referenced: orphan it (remove ownership, keep the file)
        diesel::update(images::table)
            .filter(images::id.eq(image_id))
            .set(images::owner_id.eq::<Option<i64>>(None))
            .execute(&mut conn)
            .await?;
        // Return 204 — the caller no longer owns the image, but the file stays
        // on disk for the recipes that reference it
        return Ok(StatusCode::NO_CONTENT);
    }

    // Not referenced: delete the row and the file
    diesel::delete(images::table)
        .filter(images::id.eq(image_id))
        .execute(&mut conn)
        .await?;
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

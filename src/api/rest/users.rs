use axum::{
    Json, Router,
    body::Bytes,
    extract::{Extension, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, put},
};

use crate::{
    AppState,
    error::AppError,
    models::user::{
        ProfilePictureResponse, User, UserPutPayload, UserSearchItem, UserSearchQuery,
        UserSearchResponse, UserUpdatePayload,
    },
    services::storage,
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::schema::users;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me", get(get_me).patch(patch_me).put(put_me))
        .route(
            "/me/profile-picture",
            put(update_profile_picture)
                .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024))
                .delete(delete_profile_picture),
        )
        .route("/search", get(search_users))
}

async fn get_me(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
) -> Result<Json<User>, AppError> {
    let mut conn = state.pool.get().await?;

    let user = users::table
        .filter(users::id.eq(user_id))
        .select((users::id, users::name, users::email, users::profile_picture))
        .first::<(i64, String, String, Option<String>)>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    Ok(Json(User {
        id: user.0,
        name: user.1,
        email: user.2,
        profile_picture: user.3,
    }))
}

async fn patch_me(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Json(payload): Json<UserUpdatePayload>,
) -> Result<Json<User>, AppError> {
    if let Some(name) = &payload.name
        && name.trim().is_empty()
    {
        return Err(AppError::Unprocessable("name cannot be empty".to_string()));
    }

    if let Some(email) = &payload.email
        && email.trim().is_empty()
    {
        return Err(AppError::Unprocessable("email cannot be empty".to_string()));
    }

    let mut conn = state.pool.get().await?;

    // Read current values for the COALESCE-like partial update
    let (current_name, current_email): (String, String) = users::table
        .filter(users::id.eq(user_id))
        .select((users::name, users::email))
        .first(&mut conn)
        .await?;

    let new_name = payload.name.unwrap_or(current_name);
    let new_email = payload.email.unwrap_or(current_email);

    let user = diesel::update(users::table)
        .filter(users::id.eq(user_id))
        .set((users::name.eq(new_name), users::email.eq(new_email)))
        .returning((users::id, users::name, users::email, users::profile_picture))
        .get_result::<(i64, String, String, Option<String>)>(&mut conn)
        .await?;

    Ok(Json(User {
        id: user.0,
        name: user.1,
        email: user.2,
        profile_picture: user.3,
    }))
}

async fn put_me(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Json(payload): Json<UserPutPayload>,
) -> Result<Json<User>, AppError> {
    if payload.name.trim().is_empty() {
        return Err(AppError::Unprocessable("name cannot be empty".to_string()));
    }
    if payload.email.trim().is_empty() {
        return Err(AppError::Unprocessable("email cannot be empty".to_string()));
    }

    let mut conn = state.pool.get().await?;

    let user = diesel::update(users::table)
        .filter(users::id.eq(user_id))
        .set((
            users::name.eq(payload.name.clone()),
            users::email.eq(payload.email.clone()),
        ))
        .returning((users::id, users::name, users::email, users::profile_picture))
        .get_result::<(i64, String, String, Option<String>)>(&mut conn)
        .await?;

    Ok(Json(User {
        id: user.0,
        name: user.1,
        email: user.2,
        profile_picture: user.3,
    }))
}

async fn update_profile_picture(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    bytes: Bytes,
) -> Result<Json<ProfilePictureResponse>, AppError> {
    if bytes.is_empty() {
        return Err(AppError::Unprocessable("Empty body".to_string()));
    }

    let ct = crate::api::rest::images::infer_content_type(&bytes);
    if ct == "application/octet-stream" || ct == "image/gif" {
        return Err(AppError::Unprocessable(
            "Invalid image format. Only JPEG, PNG, and WEBP are allowed".to_string(),
        ));
    }

    let hash = storage::save_image(&state.image_storage_path, &bytes).await?;

    let mut conn = state.pool.get().await?;

    // Get old hash so we can delete the old file
    let old_hash: Option<String> = users::table
        .filter(users::id.eq(user_id))
        .select(users::profile_picture)
        .first::<Option<String>>(&mut conn)
        .await?;

    diesel::update(users::table)
        .filter(users::id.eq(user_id))
        .set(users::profile_picture.eq(hash.clone()))
        .execute(&mut conn)
        .await?;

    // Delete the old file if it exists and is different from the new hash
    if let Some(old_hash) = old_hash
        && old_hash != hash
    {
        let _ = storage::delete_image(&state.image_storage_path, &old_hash).await;
    }

    Ok(Json(ProfilePictureResponse {
        profile_picture: hash,
    }))
}

async fn delete_profile_picture(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.pool.get().await?;

    // Get the current hash so we can delete the file
    let current_hash: Option<String> = users::table
        .filter(users::id.eq(user_id))
        .select(users::profile_picture)
        .first::<Option<String>>(&mut conn)
        .await?;

    diesel::update(users::table)
        .filter(users::id.eq(user_id))
        .set(users::profile_picture.eq::<Option<String>>(None))
        .execute(&mut conn)
        .await?;

    // Delete the file from disk
    if let Some(current_hash) = current_hash {
        let _ = storage::delete_image(&state.image_storage_path, &current_hash).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn search_users(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Query(query): Query<UserSearchQuery>,
) -> Result<Json<UserSearchResponse>, AppError> {
    if query.q.trim().is_empty() {
        return Err(AppError::Unprocessable("q cannot be empty".to_string()));
    }

    let limit = query.limit.unwrap_or(10).clamp(1, 50);
    // Escape LIKE wildcards so user input can't broaden the match
    let escaped = query
        .q
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{}%", escaped);

    let mut conn = state.pool.get().await?;

    let users = users::table
        .filter(users::id.ne(user_id))
        .filter(users::name.ilike(pattern))
        .select((users::id, users::name, users::profile_picture))
        .limit(limit)
        .load::<(i64, String, Option<String>)>(&mut conn)
        .await?;

    let data = users
        .into_iter()
        .map(|(id, name, profile_picture)| UserSearchItem {
            id,
            name,
            profile_picture,
        })
        .collect();

    Ok(Json(UserSearchResponse { data }))
}

// Unit tests for this module live in tests/users_validation_tests.rs —
// moved out of src/ so CodeQL does not flag test fixtures inside
// production source files.

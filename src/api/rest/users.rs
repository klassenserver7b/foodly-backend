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
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use crate::schema::{images, users};

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

    let hash = storage::hash_bytes(&bytes);

    let mut conn = state.pool.get().await?;

    // Insert the `images` row alongside the profile-picture update so the
    // hash is reachable through `GET /images/:hash` too. The row and the
    // pointer must not diverge.
    let new_hash = hash.clone();
    let storage_dir = state.image_storage_path.clone();
    let body = bytes.clone();
    conn.transaction::<_, AppError, _>(async move |conn| {
        // Serialize with every other mutation of the same hash. The bytes are
        // (re)written after the lock so a concurrent delete cannot leave the
        // new row pointing at a removed file.
        crate::db::lock_image_hash(conn, &new_hash).await?;
        storage::write_image(&storage_dir, &new_hash, &body).await?;

        // Re-uploading identical bytes is idempotent: keep the existing row
        // (and its owner) just like `upload_image` does.
        diesel::insert_into(images::table)
            .values((
                images::hash.eq(&new_hash),
                images::name.eq::<Option<String>>(None),
                images::owner_id.eq(user_id),
            ))
            .on_conflict(images::hash)
            .do_nothing()
            .execute(conn)
            .await?;

        let old_hash: Option<String> = users::table
            .filter(users::id.eq(user_id))
            .select(users::profile_picture)
            .first::<Option<String>>(conn)
            .await?;

        diesel::update(users::table)
            .filter(users::id.eq(user_id))
            .set(users::profile_picture.eq(&new_hash))
            .execute(conn)
            .await?;

        // Only remove the old file if nothing still references it. The
        // reference check and the unlink run under the old hash's lock so a
        // concurrent upload cannot re-create the row before the file goes.
        if let Some(old_hash) = old_hash
            && old_hash != new_hash
        {
            crate::db::lock_image_hash(conn, &old_hash).await?;
            if !is_image_still_referenced(conn, &old_hash, user_id).await? {
                let _ = storage::delete_image(&storage_dir, &old_hash).await;
            }
        }

        Ok(())
    })
    .await?;

    Ok(Json(ProfilePictureResponse {
        profile_picture: hash,
    }))
}

/// Check whether the image file for `hash` is still referenced by anyone:
/// - as another user's profile picture
/// - by a row in `images` (which `GET /images/:hash` serves, and which recipe
///   images/main images point at)
/// - as a recipe gallery image (`recipe_images.image_id` → `images.id`)
/// - as a recipe main image (`recipes.main_image_id` → `images.id`)
///
/// If true, the file must not be deleted from disk.
async fn is_image_still_referenced(
    conn: &mut AsyncPgConnection,
    hash: &str,
    exclude_user_id: i64,
) -> Result<bool, AppError> {
    // Another user still uses this hash as their profile picture
    let profile_used: bool = diesel::select(diesel::dsl::exists(
        users::table
            .filter(users::id.ne(exclude_user_id))
            .filter(users::profile_picture.eq(hash)),
    ))
    .first(conn)
    .await?;
    if profile_used {
        return Ok(true);
    }

    // An `images` row is itself a live reference: the file stays reachable via
    // `GET /images/:hash`, and recipe gallery/main image links point at this
    // row (`ON DELETE` on `recipe_images.image_id` is CASCADE). Deleting the
    // file here would leave the row unreadable.
    let image_exists: bool = diesel::select(diesel::dsl::exists(
        images::table.filter(images::hash.eq(hash)),
    ))
    .first(conn)
    .await?;
    if image_exists {
        return Ok(true);
    }

    Ok(false)
}

async fn delete_profile_picture(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.pool.get().await?;

    // Clearing the pointer and deciding whether the file is still referenced
    // must happen atomically under the hash's lock; otherwise a concurrent
    // upload could re-create the row after our check but before the delete.
    let storage_dir = state.image_storage_path.clone();
    conn.transaction::<_, AppError, _>(async move |conn| {
        let current_hash: Option<String> = users::table
            .filter(users::id.eq(user_id))
            .select(users::profile_picture)
            .first::<Option<String>>(conn)
            .await?;

        let Some(current_hash) = current_hash else {
            return Ok(());
        };

        crate::db::lock_image_hash(conn, &current_hash).await?;

        // Re-read under the lock: another request may have swapped the
        // picture while we waited, in which case it owns the cleanup.
        let latest_hash: Option<String> = users::table
            .filter(users::id.eq(user_id))
            .select(users::profile_picture)
            .first::<Option<String>>(conn)
            .await?;
        if latest_hash.as_deref() != Some(current_hash.as_str()) {
            return Ok(());
        }

        diesel::update(users::table)
            .filter(users::id.eq(user_id))
            .set(users::profile_picture.eq::<Option<String>>(None))
            .execute(conn)
            .await?;

        // Delete the file from disk only if nothing still references it,
        // while the lock is still held.
        if !is_image_still_referenced(conn, &current_hash, user_id).await? {
            let _ = storage::delete_image(&storage_dir, &current_hash).await;
        }

        Ok(())
    })
    .await?;

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

#[cfg(test)]
mod tests {
    use crate::models::user::{UserPutPayload, UserUpdatePayload};
    use crate::test_secrets::{TEST_EMAIL, TEST_NAME};

    #[tokio::test]
    async fn test_patch_me_validation() {
        let payload = UserUpdatePayload {
            name: Some("".to_string()),
            email: None,
        };
        assert!(payload.name.as_ref().unwrap().trim().is_empty());
    }

    #[tokio::test]
    async fn test_put_me_validation() {
        let payload = UserPutPayload {
            name: TEST_NAME.to_string(),
            email: TEST_EMAIL.to_string(),
        };
        assert!(!payload.name.trim().is_empty());
    }
}

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
    Extension(user_id): Extension<i32>,
) -> Result<Json<User>, AppError> {
    let user = sqlx::query_as!(
        User,
        r#"
        SELECT id, name, email, profile_picture
        FROM users
        WHERE id = $1
        "#,
        user_id as i64
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    Ok(Json(user))
}

async fn patch_me(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
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

    let name = payload.name;
    let email = payload.email;

    let user = sqlx::query_as!(
        User,
        r#"
        UPDATE users
        SET name = COALESCE($1, name), email = COALESCE($2, email)
        WHERE id = $3
        RETURNING id, name, email, profile_picture
        "#,
        name,
        email,
        user_id as i64
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(user))
}

async fn put_me(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Json(payload): Json<UserPutPayload>,
) -> Result<Json<User>, AppError> {
    if payload.name.trim().is_empty() {
        return Err(AppError::Unprocessable("name cannot be empty".to_string()));
    }

    if payload.email.trim().is_empty() {
        return Err(AppError::Unprocessable("email cannot be empty".to_string()));
    }

    let user = sqlx::query_as!(
        User,
        r#"
        UPDATE users
        SET name = $1, email = $2
        WHERE id = $3
        RETURNING id, name, email, profile_picture
        "#,
        payload.name,
        payload.email,
        user_id as i64
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(user))
}

async fn update_profile_picture(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
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

    sqlx::query!(
        r#"
        UPDATE users
        SET profile_picture = $1
        WHERE id = $2
        "#,
        hash,
        user_id as i64
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(ProfilePictureResponse {
        profile_picture: hash,
    }))
}

async fn delete_profile_picture(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
) -> Result<impl IntoResponse, AppError> {
    sqlx::query!(
        r#"
        UPDATE users
        SET profile_picture = NULL
        WHERE id = $1
        "#,
        user_id as i64
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn search_users(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Query(query): Query<UserSearchQuery>,
) -> Result<Json<UserSearchResponse>, AppError> {
    if query.q.trim().is_empty() {
        return Err(AppError::Unprocessable("q cannot be empty".to_string()));
    }

    let limit = query.limit.unwrap_or(10).clamp(1, 50);

    let users = sqlx::query_as!(
        UserSearchItem,
        r#"
        SELECT id, name, profile_picture
        FROM users
        WHERE id != $1 AND name ILIKE '%' || $2 || '%'
        LIMIT $3
        "#,
        user_id as i64,
        query.q,
        limit
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(UserSearchResponse { data: users }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use sqlx::PgPool;
    use std::path::PathBuf;

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_get_me(pool: PgPool) {
        let state = State(AppState {
            pool: pool.clone(),
            image_storage_path: PathBuf::from("/tmp"),
        });
        let user_id_i32 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('test_me', 'me@test.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id as i32;
        let ext = Extension(user_id_i32);

        let res = get_me(state, ext).await.unwrap();
        assert_eq!(res.0.name, "test_me");
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_patch_me(pool: PgPool) {
        let state = State(AppState {
            pool: pool.clone(),
            image_storage_path: PathBuf::from("/tmp"),
        });
        let user_id_i32 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('test_patch', 'patch@test.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id as i32;
        let ext = Extension(user_id_i32);

        let payload = Json(UserUpdatePayload {
            name: None,
            email: Some("new_patch@test.com".to_string()),
        });
        let res = patch_me(state, ext, payload).await.unwrap();
        assert_eq!(res.0.name, "test_patch");
        assert_eq!(res.0.email, "new_patch@test.com");
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_put_me(pool: PgPool) {
        let state = State(AppState {
            pool: pool.clone(),
            image_storage_path: PathBuf::from("/tmp"),
        });
        let user_id_i32 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('test_put', 'put@test.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id as i32;
        let ext = Extension(user_id_i32);

        let payload = Json(UserPutPayload {
            name: "put name".to_string(),
            email: "put@test.com".to_string(),
        });
        let res = put_me(state, ext, payload).await.unwrap();
        assert_eq!(res.0.name, "put name");
        assert_eq!(res.0.email, "put@test.com");
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_profile_picture_lifecycle(pool: PgPool) {
        let tmp = std::env::temp_dir().join("foodly_test_images");
        let state = State(AppState {
            pool: pool.clone(),
            image_storage_path: tmp.clone(),
        });
        let user_id_i32 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('test_pic', 'pic@test.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id as i32;
        let ext = Extension(user_id_i32);

        let bytes = Bytes::from(vec![0xFF, 0xD8, 0xFF, 0x01, 0x02, 0x03]); // Fake JPEG
        let res = update_profile_picture(state.clone(), ext.clone(), bytes)
            .await
            .unwrap();
        assert_eq!(res.0.profile_picture.len(), 64);

        let del_res = delete_profile_picture(state, ext).await.unwrap();
        assert_eq!(del_res.into_response().status(), StatusCode::NO_CONTENT);

        let _ = tokio::fs::remove_dir_all(tmp).await;
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_search_users(pool: PgPool) {
        let state = State(AppState {
            pool: pool.clone(),
            image_storage_path: PathBuf::from("/tmp"),
        });
        let caller_id = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('caller', 'call@test.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id as i32;
        let _ = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('Mara', 'mara@test.com', 'hash')")
            .execute(&pool).await.unwrap();
        let ext = Extension(caller_id);

        let query = Query(UserSearchQuery {
            q: "Mara".to_string(),
            limit: None,
        });
        let res = search_users(state, ext, query).await.unwrap();
        assert_eq!(res.0.data.len(), 1);
        assert_eq!(res.0.data[0].name, "Mara");
    }
}

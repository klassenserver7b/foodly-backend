//! Authentication endpoints: register, login, refresh, logout, sessions.
//!
//! Public endpoints (`register`, `login`, `refresh`) are NOT wrapped by
//! the auth middleware. Protected endpoints (`logout`, `sessions`) are.

use axum::{
    Extension, Json, Router,
    extract::State,
    http::StatusCode,
    routing::{delete, post},
};
use diesel::dsl::sql;
use diesel::prelude::*;
use diesel::sql_types::{Bool, Text};
use diesel_async::RunQueryDsl;

use crate::{
    AppState,
    error::AppError,
    models::auth::{
        LoginPayload, RefreshPayload, RegisterPayload, TokenPairResponse, UserWithTokens,
    },
    schema::{refresh_tokens, users},
    services::auth::{
        create_access_token, generate_refresh_token, hash_password, hash_token, jwt_expiry_minutes,
        jwt_secret, refresh_token_expiry_days, verify_password,
    },
};

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/refresh", post(refresh))
}

pub fn protected_router() -> Router<AppState> {
    Router::new()
        .route("/logout", post(logout))
        .route("/sessions", delete(sessions))
}

/// A valid argon2id hash of a throwaway password, used to equalize the
/// timing of the user-not-found path in `login` so response-time
/// differences do not leak whether an email exists.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$nhLgEhJ6GGcRqjzCh2Cibw$qMEyg1seXV3uM6myzINmp1SF2iRCSlChggiIXbdCLp4";

// ─── Register ──────────────────────────────────────────────────────────

async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterPayload>,
) -> Result<(StatusCode, Json<UserWithTokens>), AppError> {
    if payload.name.trim().is_empty() {
        return Err(AppError::Unprocessable("name cannot be empty".into()));
    }
    if payload.email.trim().is_empty() {
        return Err(AppError::Unprocessable("email cannot be empty".into()));
    }
    if payload.password.len() < 8 {
        return Err(AppError::Unprocessable(
            "password must be at least 8 characters".into(),
        ));
    }

    let hashed = hash_password(&payload.password)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("hash: {e}")))?;

    let mut conn = state.pool.get().await?;

    // No pre-check: the CITEXT UNIQUE index on email enforces this.
    // A case-variant duplicate (e.g. USER@ vs user@) hits the same
    // unique index, so both paths return the same 409 "Duplicate value"
    // from the UniqueViolation mapping in error.rs.

    let inserted: (i64, String, String, Option<String>) = diesel::insert_into(users::table)
        .values((
            users::name.eq(payload.name.trim()),
            users::email.eq(payload.email.trim()),
            users::password_hash.eq(hashed),
            users::profile_picture.eq(Option::<String>::None),
        ))
        .returning((users::id, users::name, users::email, users::profile_picture))
        .get_result(&mut conn)
        .await?;

    // Issue tokens
    let secret = jwt_secret();
    let access = create_access_token(inserted.0, secret, jwt_expiry_minutes())?;
    let refresh = generate_refresh_token();

    // Store refresh token hash
    let refresh_hash = hash_token(&refresh);
    let expires_at = chrono::Utc::now() + chrono::Duration::days(refresh_token_expiry_days());
    diesel::insert_into(refresh_tokens::table)
        .values((
            refresh_tokens::user_id.eq(inserted.0),
            refresh_tokens::token_hash.eq(&refresh_hash),
            refresh_tokens::expires_at.eq(expires_at.naive_utc()),
            refresh_tokens::created_at.eq(chrono::Utc::now().naive_utc()),
        ))
        .execute(&mut conn)
        .await?;

    let user = crate::models::user::User {
        id: inserted.0,
        name: inserted.1,
        email: inserted.2,
        profile_picture: inserted.3,
    };

    Ok((
        StatusCode::CREATED,
        Json(UserWithTokens {
            user,
            access_token: access,
            refresh_token: refresh,
        }),
    ))
}

// ─── Login ─────────────────────────────────────────────────────────────

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginPayload>,
) -> Result<Json<UserWithTokens>, AppError> {
    let mut conn = state.pool.get().await?;

    let user: Option<(i64, String, String, String)> = users::table
        .filter(
            sql::<Bool>("users.email = ")
                .bind::<Text, _>(payload.email.trim())
                .sql("::citext"),
        )
        .select((users::id, users::name, users::email, users::password_hash))
        .first::<(i64, String, String, String)>(&mut conn)
        .await
        .optional()?;

    let Some((id, name, email, password_hash)) = user else {
        // User not found — verify against dummy hash for timing equalization
        let _ = verify_password(&payload.password, DUMMY_HASH);
        return Err(AppError::Unauthorized("invalid credentials".into()));
    };

    if !verify_password(&payload.password, &password_hash).unwrap_or(false) {
        return Err(AppError::Unauthorized("invalid credentials".into()));
    }

    let secret = jwt_secret();
    let access = create_access_token(id, secret, jwt_expiry_minutes())?;
    let refresh = generate_refresh_token();

    // Store refresh token hash
    let refresh_hash = hash_token(&refresh);
    let expires_at = chrono::Utc::now() + chrono::Duration::days(refresh_token_expiry_days());
    diesel::insert_into(refresh_tokens::table)
        .values((
            refresh_tokens::user_id.eq(id),
            refresh_tokens::token_hash.eq(&refresh_hash),
            refresh_tokens::expires_at.eq(expires_at.naive_utc()),
            refresh_tokens::created_at.eq(chrono::Utc::now().naive_utc()),
        ))
        .execute(&mut conn)
        .await?;

    let user_model = crate::models::user::User {
        id,
        name,
        email,
        profile_picture: None,
    };

    Ok(Json(UserWithTokens {
        user: user_model,
        access_token: access,
        refresh_token: refresh,
    }))
}

// ─── Refresh ───────────────────────────────────────────────────────────

async fn refresh(
    State(state): State<AppState>,
    Json(payload): Json<RefreshPayload>,
) -> Result<Json<TokenPairResponse>, AppError> {
    let mut conn = state.pool.get().await?;

    let token_hash = hash_token(&payload.refresh_token);
    let now = chrono::Utc::now();

    // Atomic rotation: DELETE ... RETURNING in a single statement.
    // A concurrent refresh with the same token sees zero rows and 401s.
    let stored: Option<(i64, chrono::NaiveDateTime)> = diesel::delete(refresh_tokens::table)
        .filter(refresh_tokens::token_hash.eq(&token_hash))
        .returning((refresh_tokens::user_id, refresh_tokens::expires_at))
        .get_result(&mut conn)
        .await
        .optional()?;

    let Some((user_id, expires_at)) = stored else {
        return Err(AppError::Unauthorized("invalid refresh token".into()));
    };

    if expires_at < now.naive_utc() {
        return Err(AppError::Unauthorized("invalid refresh token".into()));
    }

    let secret = jwt_secret();
    let access = create_access_token(user_id, secret, jwt_expiry_minutes())?;
    let new_refresh = generate_refresh_token();

    // Insert new refresh token
    let new_hash = hash_token(&new_refresh);
    let new_expires = chrono::Utc::now() + chrono::Duration::days(refresh_token_expiry_days());
    diesel::insert_into(refresh_tokens::table)
        .values((
            refresh_tokens::user_id.eq(user_id),
            refresh_tokens::token_hash.eq(&new_hash),
            refresh_tokens::expires_at.eq(new_expires.naive_utc()),
            refresh_tokens::created_at.eq(chrono::Utc::now().naive_utc()),
        ))
        .execute(&mut conn)
        .await?;

    Ok(Json(TokenPairResponse {
        access_token: access,
        refresh_token: new_refresh,
    }))
}

// ─── Logout ────────────────────────────────────────────────────────────

async fn logout(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Json(payload): Json<RefreshPayload>,
) -> Result<StatusCode, AppError> {
    let mut conn = state.pool.get().await?;

    let token_hash = hash_token(&payload.refresh_token);
    // Logout deletes the refresh token by hash AND user_id scope
    diesel::delete(refresh_tokens::table)
        .filter(refresh_tokens::token_hash.eq(&token_hash))
        .filter(refresh_tokens::user_id.eq(user_id))
        .execute(&mut conn)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ─── Sessions (logout all) ─────────────────────────────────────────────

async fn sessions(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
) -> Result<StatusCode, AppError> {
    let mut conn = state.pool.get().await?;

    diesel::delete(refresh_tokens::table)
        .filter(refresh_tokens::user_id.eq(user_id))
        .execute(&mut conn)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

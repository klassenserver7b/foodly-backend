//! Integration tests for the REST API.
//!
//! Each test gets a fresh, isolated database. The harness connects to the
//! admin database, creates a uniquely-named test database, applies the
//! embedded migrations, and hands back a diesel-async pool for it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::{AsyncConnection, AsyncPgConnection, SimpleAsyncConnection};
use foodly_backend::db::DbPool;
use foodly_backend::{AppState, app};
use http_body_util::BodyExt; // for `collect`
use serde_json::Value;
use tower::ServiceExt; // for `oneshot`

static DB_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Guards the one-time process-wide env mutation so parallel `#[tokio::test]`
/// cases can't race each other while writing the environment.
static ENV_SETUP: std::sync::Once = std::sync::Once::new();

/// Create an isolated test database (with migrations applied) and return a
/// pool connected to it. Requires a reachable PostgreSQL at the base URL.
async fn test_pool() -> DbPool {
    ENV_SETUP.call_once(|| {
        // Disable rate limiting for tests — oneshot() requests lack the
        // ConnectInfo<SocketAddr> extension the governor key extractor needs.
        // SAFETY: this runs exactly once before any test body uses the env,
        // guarded by `ENV_SETUP`.
        unsafe {
            std::env::set_var("RATE_LIMIT_MODE", "disabled");
        }
        // Ensure a JWT secret is set for auth endpoint tests. This is a
        // deterministic test secret, not a production secret.
        if std::env::var("JWT_SECRET").is_err() {
            unsafe {
                std::env::set_var("JWT_SECRET", "foodly-test-jwt-secret-0123456789abcdef");
            }
        }
        // Ensure a pepper is set for auth endpoint tests. This is a
        // deterministic test pepper, not a production pepper.
        if std::env::var("PASSWORD_PEPPER").is_err() {
            unsafe {
                std::env::set_var("PASSWORD_PEPPER", "foodly-test-pepper-0123456789abcdef");
            }
        }
        let _ = dotenvy::dotenv();
    });
    let base = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let user = std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string());
        let password =
            std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "password".to_string());
        let host = std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port = std::env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
        format!("postgres://{user}:{password}@{host}:{port}/foodly")
    });

    // The admin database, used only to create/drop the per-test database.
    let admin_url = match base.rsplit_once('/') {
        Some((prefix, _db)) => format!("{prefix}/postgres"),
        None => base.clone(),
    };

    let n = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let test_db = format!("foodly_test_{}_{}", std::process::id(), n);
    let test_url = match base.rsplit_once('/') {
        Some((prefix, _db)) => format!("{prefix}/{test_db}"),
        None => base.clone(),
    };

    {
        let mut admin = AsyncPgConnection::establish(&admin_url).await.unwrap();
        admin
            .batch_execute(&format!("DROP DATABASE IF EXISTS {test_db}"))
            .await
            .unwrap();
        admin
            .batch_execute(&format!("CREATE DATABASE {test_db}"))
            .await
            .unwrap();
    }

    let config = AsyncDieselConnectionManager::<AsyncPgConnection>::new(test_url);
    let pool = diesel_async::pooled_connection::deadpool::Pool::builder(config)
        .build()
        .unwrap();

    foodly_backend::db::run_migrations(&pool).await.unwrap();
    pool
}

async fn test_state() -> AppState {
    AppState {
        pool: test_pool().await,
        image_storage_path: PathBuf::from("/tmp"),
    }
}

#[tokio::test]
async fn test_unauthorized_access() {
    let app = app(test_state().await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tags")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_authorized_get_tags() {
    let app = app(test_state().await);

    // Register a user to get a valid JWT
    let (access_token, _refresh) =
        register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tags")
                .header(header::AUTHORIZATION, format!("Bearer {}", access_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert!(json.get("data").is_some());
    assert!(json["data"].is_array());
}

#[tokio::test]
async fn test_create_recipe_http() {
    let state = test_state().await;
    let app = app(state.clone());

    // Register a user to get a valid JWT
    let (access_token, _refresh_token) =
        register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let payload = serde_json::json!({
        "name": "Integration Recipe",
        "tags": [],
        "source": null,
        "time": null,
        "workMinutes": 25,
        "overallMinutes": 50,
        "sizeNumber": 4,
        "sizeText": "portions",
        "notes": [],
        "mainImage": null,
        "images": [],
        "sections": []
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/recipes")
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["name"].as_str().unwrap(), "Integration Recipe");
}

/// Ingredient lists are capped so a single recipe body cannot trigger a
/// huge multi-row insert (previously one round-trip per ingredient).
#[tokio::test]
async fn test_create_recipe_rejects_too_many_ingredients() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Bulk User", "bulk@example.com", "password123").await;

    // 1001 ingredients — one more than MAX_RECIPE_ITEMS.
    let ingredients: Vec<Value> = (0..=1000)
        .map(|i| serde_json::json!({ "text": format!("ingredient {i}") }))
        .collect();

    let payload = serde_json::json!({
        "name": "Huge Recipe",
        "sections": [{ "name": null, "ingredients": ingredients, "steps": [] }]
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/recipes")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["error"]["message"], "sections accept at most 1000 ingredients in total",
        "over-cap ingredient list must be rejected with a clear message"
    );
}

// ─── Auth tests ─────────────────────────────────────────────────────────

/// Helper: register a user and return (access_token, refresh_token).
async fn register_test_user(
    app: &axum::Router,
    name: &str,
    email: &str,
    password: &str,
) -> (String, String) {
    let payload = serde_json::json!({
        "name": name,
        "email": email,
        "password": password,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED, "register failed");

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let access = json["accessToken"].as_str().unwrap().to_string();
    let refresh = json["refreshToken"].as_str().unwrap().to_string();
    (access, refresh)
}

/// Helper: login and return (access_token, refresh_token).
async fn login_test_user(app: &axum::Router, email: &str, password: &str) -> (String, String) {
    let payload = serde_json::json!({
        "email": email,
        "password": password,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK, "login failed");

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let access = json["accessToken"].as_str().unwrap().to_string();
    let refresh = json["refreshToken"].as_str().unwrap().to_string();
    (access, refresh)
}

#[tokio::test]
async fn test_register_user() {
    let app = app(test_state().await);

    let (access, refresh) =
        register_test_user(&app, "Test User", "test@example.com", "password123").await;

    assert!(!access.is_empty());
    assert!(!refresh.is_empty());
}

#[tokio::test]
async fn test_login_user() {
    let app = app(test_state().await);
    register_test_user(&app, "Test User", "test@example.com", "password123").await;
    let (access2, refresh2) = login_test_user(&app, "test@example.com", "password123").await;

    assert!(!access2.is_empty());
    assert!(!refresh2.is_empty());
}

#[tokio::test]
async fn test_login_wrong_password() {
    let app = app(test_state().await);
    register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let payload = serde_json::json!({
        "email": "test@example.com",
        "password": "wrong",
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_refresh_token() {
    let app = app(test_state().await);
    let (_access, refresh) =
        register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let payload = serde_json::json!({ "refreshToken": refresh });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(!json["accessToken"].as_str().unwrap().is_empty());
    assert!(!json["refreshToken"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn test_refresh_invalid_token() {
    let app = app(test_state().await);

    let payload = serde_json::json!({ "refreshToken": "invalid" });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// AC-002: registering with an email that already exists must return 409
/// (CITEXT makes the uniqueness case-insensitive).
#[tokio::test]
async fn test_register_duplicate_email() {
    let app = app(test_state().await);
    register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let payload = serde_json::json!({
        "name": "Other User",
        "email": "TEST@EXAMPLE.COM",
        "password": "password123",
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "duplicate email (case-insensitive) must 409"
    );
}

/// Logging in with different casing must match the registered email
/// (CITEXT compares case-insensitively) and return the stored email.
#[tokio::test]
async fn test_login_email_case_insensitive() {
    let app = app(test_state().await);
    let (access, _refresh) =
        register_test_user(&app, "Test User", "Mixed.Case@Example.com", "password123").await;
    assert!(!access.is_empty());

    let payload = serde_json::json!({
        "email": "mixed.case@EXAMPLE.COM",
        "password": "password123",
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "login must succeed regardless of email casing"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["user"]["email"].as_str().unwrap(),
        "Mixed.Case@Example.com",
        "login must return the same stored email, not the input casing"
    );
}

/// AC-005 partial: a rotated (used) refresh token must 401 when replayed.
#[tokio::test]
async fn test_refresh_reuse_rotated_token() {
    let app = app(test_state().await);
    let (_access, refresh) =
        register_test_user(&app, "Test User", "test@example.com", "password123").await;

    // First refresh succeeds and rotates the token
    let payload = serde_json::json!({ "refreshToken": refresh });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Replay the SAME (now consumed) refresh token — must be rejected
    let response2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response2.status(),
        StatusCode::UNAUTHORIZED,
        "reused rotated refresh token must 401"
    );
}

// ─── Image tests ────────────────────────────────────────────────────────

/// Minimal PNG-shaped payload: the 8 magic bytes `infer_content_type`
/// sniffs, followed by arbitrary data. Not a decodable PNG — the API only
/// validates magic bytes, it does not decode the image.
fn test_png(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(payload);
    bytes
}

/// Upload `bytes` as user `token` and return the content hash.
async fn upload_test_image(app: &axum::Router, token: &str, bytes: &[u8]) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/images")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from(bytes.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "image upload failed"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    json["hash"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_image_upload_get_public_delete_owner_only() {
    let app = app(test_state().await);
    let (token, _refresh) =
        register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let bytes = test_png(b"full lifecycle image payload");

    // Upload
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/images")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from(bytes.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let hash = json["hash"].as_str().unwrap().to_string();
    assert!(json.get("id").is_some());
    assert_eq!(hash.len(), 64, "hash must be a SHA-256 hex digest");

    // Public GET — no Authorization header
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/images/{hash}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "public image GET must 200"
    );

    let headers = response.headers().clone();
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/png");
    assert_eq!(
        headers.get(header::ETAG).unwrap(),
        &format!("\"{hash}\""),
        "ETag must be the quoted hash"
    );
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), bytes.as_slice(), "body must match upload");

    // Owner DELETE
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/images/{hash}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Second DELETE — already gone
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/images/{hash}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_image_delete_non_owner_404() {
    let app = app(test_state().await);
    let (token_a, _) = register_test_user(&app, "User A", "a@example.com", "password123").await;
    let (token_b, _) = register_test_user(&app, "User B", "b@example.com", "password123").await;

    let bytes = test_png(b"non-owner delete image payload");
    let hash = upload_test_image(&app, &token_a, &bytes).await;

    // Non-owner DELETE must 404 — no existence leak.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/images/{hash}"))
                .header(header::AUTHORIZATION, format!("Bearer {token_b}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "non-owner must not be able to delete"
    );

    // Owner can still delete it.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/images/{hash}"))
                .header(header::AUTHORIZATION, format!("Bearer {token_a}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_image_upload_requires_auth() {
    let app = app(test_state().await);
    let bytes = test_png(b"unauthenticated upload payload");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/images")
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_cors_preflight_options() {
    let app = app(test_state().await);

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/auth/register")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "authorization, content-type",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let headers = response.headers();
    assert_eq!(
        headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "http://localhost:5173"
    );

    let allow_methods = headers
        .get(header::ACCESS_CONTROL_ALLOW_METHODS)
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        allow_methods.contains("post"),
        "preflight must allow POST, got {allow_methods:?}"
    );

    let allow_headers = headers
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        allow_headers.contains("authorization") && allow_headers.contains("content-type"),
        "preflight must allow authorization + content-type, got {allow_headers:?}"
    );
}

#[tokio::test]
async fn test_register_weak_password_422() {
    let app = app(test_state().await);

    let payload = serde_json::json!({
        "name": "T",
        "email": "t@t.com",
        "password": "short",
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "password shorter than 8 characters must 422"
    );
}

// ─── Bulk user lookup (issue #10) ───────────────────────────────────────

/// Helper: GET `/users/me` and return the caller's user id.
async fn me_id(app: &axum::Router, token: &str) -> i64 {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/users/me")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK, "GET /users/me failed");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    json["id"].as_i64().unwrap()
}

/// Helper: GET `/users?ids=...` and return the parsed body.
async fn bulk_users(app: &axum::Router, token: &str, ids: &str) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/users?ids={ids}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK, "bulk lookup failed");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_bulk_get_users_rejects_too_many_ids() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Owner", "owner@example.com", "password123").await;

    // One more than MAX_IDS (100).
    let ids: Vec<String> = (1..=101).map(|i| i.to_string()).collect();
    let query = ids.join(",");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/users?ids={query}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["message"], "ids accepts at most 100 entries");
}

#[tokio::test]
async fn test_bulk_get_users() {
    let app = app(test_state().await);

    let (token_a, _) = register_test_user(&app, "User A", "a@example.com", "password123").await;
    let (token_b, _) = register_test_user(&app, "User B", "b@example.com", "password123").await;
    let (_token_c, _) = register_test_user(&app, "User C", "c@example.com", "password123").await;

    let id_a = me_id(&app, &token_a).await;
    let id_b = me_id(&app, &token_b).await;

    let json = bulk_users(&app, &token_a, &format!("{id_a},{id_b}")).await;
    let data = json["data"].as_array().expect("data must be an array");
    assert_eq!(data.len(), 2, "only the requested users must be returned");

    let mut ids: Vec<i64> = data.iter().map(|u| u["id"].as_i64().unwrap()).collect();
    ids.sort_unstable();
    let mut expected = vec![id_a, id_b];
    expected.sort_unstable();
    assert_eq!(ids, expected);

    for user in data {
        assert!(user["name"].is_string(), "name must be present");
        assert!(
            user.get("profilePicture").is_some(),
            "profilePicture (camelCase) must be present"
        );
        assert!(
            user.get("email").is_none(),
            "bulk lookup must not leak email addresses"
        );
    }
}

#[tokio::test]
async fn test_bulk_get_users_drops_malformed_ids() {
    let app = app(test_state().await);

    let (token_a, _) = register_test_user(&app, "User A", "a@example.com", "password123").await;
    let (token_b, _) = register_test_user(&app, "User B", "b@example.com", "password123").await;

    let id_a = me_id(&app, &token_a).await;
    let id_b = me_id(&app, &token_b).await;

    let json = bulk_users(&app, &token_a, &format!("{id_a},abc,{id_b}")).await;
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2, "malformed segment must be dropped silently");

    let mut ids: Vec<i64> = data.iter().map(|u| u["id"].as_i64().unwrap()).collect();
    ids.sort_unstable();
    let mut expected = vec![id_a, id_b];
    expected.sort_unstable();
    assert_eq!(ids, expected);
}

#[tokio::test]
async fn test_bulk_get_users_empty_ids() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "User A", "a@example.com", "password123").await;

    let json = bulk_users(&app, &token, "").await;
    assert_eq!(json["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_bulk_get_users_rejects_oversized_ids() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "User A", "a@example.com", "password123").await;

    // All-malformed segments used to bypass the entry cap and force a full
    // split/parse of the raw string; the byte cap must reject it up front.
    let ids = vec!["abc"; 2000].join(",");
    assert!(ids.len() > 2048);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/users?ids={ids}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// ─── Recipe access-since (issue #14) ────────────────────────────────────

/// Helper: create a minimal recipe as `token` and return the full response.
async fn create_test_recipe(app: &axum::Router, token: &str, name: &str) -> Value {
    let payload = serde_json::json!({
        "name": name,
        "tags": [],
        "source": null,
        "time": null,
        "workMinutes": null,
        "overallMinutes": null,
        "sizeNumber": null,
        "sizeText": null,
        "notes": [],
        "mainImage": null,
        "images": [],
        "sections": []
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/recipes")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK, "recipe creation failed");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

/// Helper: GET `/recipes/:id` as `token` and return the body.
async fn get_recipe_json(app: &axum::Router, token: &str, id: i64) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/recipes/{id}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK, "GET recipe failed");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

/// Helper: add a (recipe, user) membership row to `recipe_editors` or
/// `recipe_viewers` using the table's default `added_at`.
async fn add_recipe_member(state: &AppState, table: &str, recipe_id: i64, user_id: i64) {
    let mut conn = state.pool.get().await.unwrap();
    conn.batch_execute(&format!(
        "INSERT INTO {table} (recipe_id, user_id) VALUES ({recipe_id}, {user_id})"
    ))
    .await
    .unwrap();
}

/// The owner's access starts when the recipe was created — `myAccessSince`
/// must equal `createdAt`, both on the full object and the list preview.
#[tokio::test]
async fn test_recipe_my_access_since_owner() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Owner", "owner@example.com", "password123").await;

    let recipe = create_test_recipe(&app, &token, "Owner Recipe").await;
    let recipe_id = recipe["id"].as_i64().unwrap();
    assert_eq!(
        recipe["myAccessSince"], recipe["createdAt"],
        "owner's myAccessSince must be createdAt"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/recipes")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let list: Value = serde_json::from_slice(&body).unwrap();

    let preview = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"].as_i64() == Some(recipe_id))
        .expect("recipe must be in the owner's list");
    assert_eq!(
        preview["myAccessSince"], preview["createdAt"],
        "owner's preview myAccessSince must be createdAt"
    );
}

/// Editors and viewers get the join-table `added_at`; the full fetch and the
/// list preview must both expose it.
#[tokio::test]
async fn test_recipe_my_access_since_shared() {
    let state = test_state().await;
    let app = app(state.clone());

    let (owner_token, _) =
        register_test_user(&app, "Owner", "owner@example.com", "password123").await;
    let (editor_token, _) =
        register_test_user(&app, "Editor", "editor@example.com", "password123").await;
    let (viewer_token, _) =
        register_test_user(&app, "Viewer", "viewer@example.com", "password123").await;

    let recipe = create_test_recipe(&app, &owner_token, "Shared Recipe").await;
    let recipe_id = recipe["id"].as_i64().unwrap();

    let editor_id = me_id(&app, &editor_token).await;
    let viewer_id = me_id(&app, &viewer_token).await;
    add_recipe_member(&state, "recipe_editors", recipe_id, editor_id).await;
    add_recipe_member(&state, "recipe_viewers", recipe_id, viewer_id).await;

    for token in [&editor_token, &viewer_token] {
        let json = get_recipe_json(&app, token, recipe_id).await;
        let since = json["myAccessSince"]
            .as_str()
            .expect("myAccessSince must be set for shared users");
        assert!(!since.is_empty());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/recipes")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let list: Value = serde_json::from_slice(&body).unwrap();

        let preview = list["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"].as_i64() == Some(recipe_id))
            .expect("shared recipe must appear in the user's list");
        assert_eq!(
            preview["myAccessSince"].as_str(),
            Some(since),
            "list preview must expose the same myAccessSince as the full fetch"
        );
    }
}

/// `sort.field = "access_since"` is accepted and preserves the usual
/// visibility-scoped result set.
#[tokio::test]
async fn test_recipe_search_sort_by_access_since() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Owner", "owner@example.com", "password123").await;

    let first = create_test_recipe(&app, &token, "First Recipe").await;
    let second = create_test_recipe(&app, &token, "Second Recipe").await;

    let payload = serde_json::json!({
        "sort": { "field": "access_since", "order": "desc" }
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/recipes/search")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let ids: Vec<i64> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            second["id"].as_i64().unwrap(),
            first["id"].as_i64().unwrap(),
        ],
        "descending access_since must list the most recently created recipe first"
    );
}

// ─── Categories: PUT /categories/order ────────────────────────────────────

/// Helper: create a category via `POST /api/v1/categories` and return the
/// created category JSON (including its `id`).
async fn create_test_category(app: &axum::Router, token: &str, name: &str) -> Value {
    let payload = serde_json::json!({ "name": name, "color": "#ff0000" });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/categories")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "category creation failed"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_categories_order_reorders_categories() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let cat1 = create_test_category(&app, &token, "Breakfast").await;
    let cat2 = create_test_category(&app, &token, "Dinner").await;

    // Reorder: Dinner first, Breakfast second
    let payload = serde_json::json!({ "categoryIds": [cat2["id"], cat1["id"]] });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/categories/order")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let ids: Vec<i64> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![cat2["id"].as_i64().unwrap(), cat1["id"].as_i64().unwrap(),],
        "reorder response must reflect the requested order"
    );
}

#[tokio::test]
async fn test_categories_order_rejects_too_many_ids() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Test User", "test@example.com", "password123").await;

    // 101 distinct ids — one more than MAX_REORDER_IDS
    let too_many: Vec<i64> = (1..=101).collect();
    let payload = serde_json::json!({ "categoryIds": too_many });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/categories/order")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["error"]["message"], "categoryIds accepts at most 100 entries",
        "over-cap payload must be rejected with a clear message"
    );
}

#[tokio::test]
async fn test_update_category_recipes_rejects_too_many_ids() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let cat = create_test_category(&app, &token, "Breakfast").await;
    let cat_id = cat["id"].as_i64().unwrap();

    // 501 distinct ids — one more than MAX_RECIPE_IDS
    let too_many: Vec<i64> = (1..=501).collect();
    let payload = serde_json::json!({ "recipeIds": too_many });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/categories/{}/recipes", cat_id))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["error"]["message"], "recipeIds accepts at most 500 entries",
        "over-cap payload must be rejected with a clear message"
    );
}

/// `page` is clamped server-side: an absurd page must not overflow the offset
/// computation or trigger an unbounded deep OFFSET scan.
#[tokio::test]
async fn test_listing_clamps_absurd_page() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Test User", "test@example.com", "password123").await;
    create_test_category(&app, &token, "Breakfast").await;

    for uri in [
        "/api/v1/categories?page=9223372036854775807&limit=100",
        "/api/v1/recipes?page=9223372036854775807&limit=100",
        "/api/v1/categories?page=1000000000&limit=100",
        "/api/v1/recipes?page=1000000000&limit=100",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "{uri} must not error");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["data"].as_array().map(Vec::len),
            Some(0),
            "{uri} is past the end of the data and must return an empty page"
        );
    }
}

/// Listing categories must hydrate each category's recipes in one batched
/// query — and keep the per-category position ordering intact.
#[tokio::test]
async fn test_list_categories_hydrates_recipes_per_category() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Test User", "test@example.com", "password123").await;

    let breakfast = create_test_category(&app, &token, "Breakfast").await;
    let dinner = create_test_category(&app, &token, "Dinner").await;
    let breakfast_id = breakfast["id"].as_i64().unwrap();
    let dinner_id = dinner["id"].as_i64().unwrap();

    let r1 = create_test_recipe(&app, &token, "Porridge").await["id"]
        .as_i64()
        .unwrap();
    let r2 = create_test_recipe(&app, &token, "Pancakes").await["id"]
        .as_i64()
        .unwrap();
    let r3 = create_test_recipe(&app, &token, "Stew").await["id"]
        .as_i64()
        .unwrap();

    // Assign in reverse order so the position column (not the id) decides.
    for (category_id, recipe_ids) in [
        (breakfast_id, serde_json::json!([r2, r1])),
        (dinner_id, serde_json::json!([r3])),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/categories/{category_id}/recipes"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({ "recipeIds": recipe_ids }))
                            .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "recipe assignment failed"
        );
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/categories")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let recipes_of = |id: i64| -> Vec<i64> {
        json["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"].as_i64() == Some(id))
            .unwrap_or_else(|| panic!("category {id} missing from listing"))["recipes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect()
    };

    assert_eq!(
        recipes_of(breakfast_id),
        vec![r2, r1],
        "recipes must be grouped per category and sorted by position"
    );
    assert_eq!(recipes_of(dinner_id), vec![r3]);
}

/// Filter arrays build one `EXISTS` sub-select (and bind parameter) per value,
/// so they must be capped to keep the statement inside Postgres' limits.
#[tokio::test]
async fn test_recipe_search_rejects_too_many_filter_values() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Owner", "owner@example.com", "password123").await;

    // 101 values — one more than MAX_FILTER_VALUES.
    let too_many: Vec<i64> = (1..=101).collect();
    let payload = serde_json::json!({ "filters": { "categories": too_many } });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/recipes/search")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["error"]["message"], "categories accepts at most 100 entries",
        "over-cap filter array must be rejected with a clear message"
    );
}

/// Enum filter arrays left-nest one OR per entry; diesel walks that tree
/// recursively, so a huge `accessRights` array would overflow the stack.
#[tokio::test]
async fn test_recipe_search_rejects_too_many_access_rights() {
    let app = app(test_state().await);
    let (token, _) = register_test_user(&app, "Owner", "owner@example.com", "password123").await;

    // 101 entries — one more than MAX_FILTER_VALUES and far above the enum cap.
    let too_many: Vec<&str> = (0..101).map(|_| "owner").collect();
    let payload = serde_json::json!({ "filters": { "accessRights": too_many } });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/recipes/search")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["error"]["message"], "accessRights accepts at most 10 entries",
        "over-cap enum filter array must be rejected with a clear message"
    );
}

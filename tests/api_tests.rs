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

/// Create an isolated test database (with migrations applied) and return a
/// pool connected to it. Requires a reachable PostgreSQL at the base URL.
async fn test_pool() -> DbPool {
    // Disable rate limiting for tests — oneshot() requests lack the
    // ConnectInfo<SocketAddr> extension the governor key extractor needs.
    // SAFETY: single-threaded test setup before concurrent handlers run.
    unsafe {
        std::env::set_var("RATE_LIMIT_MODE", "disabled");
    }
    // Ensure a JWT secret is set for auth endpoint tests. This is a
    // deterministic test secret, not a production secret.
    // SAFETY: Safe because we're in single-threaded test setup before
    // any test handlers run concurrently.
    if std::env::var("JWT_SECRET").is_err() {
        unsafe {
            std::env::set_var("JWT_SECRET", "foodly-test-jwt-secret-0123456789abcdef");
        }
    }
    // Ensure a pepper is set for auth endpoint tests. This is a
    // deterministic test pepper, not a production pepper.
    // SAFETY: Safe because we're in single-threaded test setup before
    // any test handlers run concurrently.
    if std::env::var("PASSWORD_PEPPER").is_err() {
        unsafe {
            std::env::set_var("PASSWORD_PEPPER", "foodly-test-pepper-0123456789abcdef");
        }
    }
    let _ = dotenvy::dotenv();
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

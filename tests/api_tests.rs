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

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tags")
                .header(header::AUTHORIZATION, "Bearer mock-token")
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

    // The auth middleware mock hardcodes user id 1; on the fresh test
    // database the first inserted user gets exactly that id.
    {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;
        use foodly_backend::schema::users;

        let mut conn = state.pool.get().await.unwrap();
        let user_id: i64 = diesel::insert_into(users::table)
            .values((
                users::name.eq("test"),
                users::email.eq("test@example.com"),
                users::password_hash.eq("hash"),
            ))
            .returning(users::id)
            .get_result(&mut conn)
            .await
            .unwrap();
        assert_eq!(user_id, 1);
    }

    let app = app(state);

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
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/recipes")
                .header(header::AUTHORIZATION, "Bearer mock-token")
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

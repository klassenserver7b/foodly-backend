use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use foodly_backend::{AppState, app};
use http_body_util::BodyExt; // for `collect`
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt; // for `oneshot`

#[sqlx::test(migrations = "src/db/migrations")]
async fn test_unauthorized_access(pool: PgPool) {
    let state = AppState {
        pool: pool.clone(),
        image_storage_path: std::path::PathBuf::from("/tmp"),
    };
    let app = app(state);

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

#[sqlx::test(migrations = "src/db/migrations")]
async fn test_authorized_get_tags(pool: PgPool) {
    let state = AppState {
        pool: pool.clone(),
        image_storage_path: std::path::PathBuf::from("/tmp"),
    };
    let app = app(state);

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

#[sqlx::test(migrations = "src/db/migrations")]
async fn test_create_recipe_http(pool: PgPool) {
    let state = AppState {
        pool: pool.clone(),
        image_storage_path: std::path::PathBuf::from("/tmp"),
    };
    let app = app(state);

    // Insert mock user directly into DB as the auth mock requires ID=1 by default
    // actually, auth mock injects `Extension(1i32)`. So we must have a user with ID=1 in the DB.
    // If not 1, we can just insert one user and use it. Wait, auth mock hardcodes `let mock_user_id = 1;`.
    // So the mock user ID in the test DB will need to be 1. The ID is IDENTITY GENERATED ALWAYS.
    // In our unit test, we retrieved the generated ID. But the HTTP layer hardcodes 1.
    // To fix this without breaking the mock, let's insert a user, it will be 1 on an empty DB.
    let user_id_i64 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('test', 'test@example.com', 'hash') RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap()
        .id;

    // In a clean test DB, this should be 1.
    assert_eq!(user_id_i64, 1);

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

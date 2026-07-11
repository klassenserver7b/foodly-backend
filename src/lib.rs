use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use tower_http::cors::{Any, CorsLayer};

pub mod api;
pub mod db;
pub mod error;
pub mod macros;
pub mod models;
pub mod services;

/// The shared application state injected into all Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub image_storage_path: std::path::PathBuf,
}

pub async fn auth_middleware(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    let auth_header = req.headers().get(header::AUTHORIZATION);

    if let Some(auth_value) = auth_header
        && let Ok(auth_str) = auth_value.to_str()
        && auth_str.starts_with("Bearer ")
    {
        let mock_user_id = 1;
        req.extensions_mut().insert(mock_user_id);
        return Ok(next.run(req).await);
    }

    Err(StatusCode::UNAUTHORIZED)
}

pub fn app(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .nest("/api/v1", api::rest::router())
        .route_layer(middleware::from_fn(auth_middleware))
        .layer(cors)
        .with_state(state)
}

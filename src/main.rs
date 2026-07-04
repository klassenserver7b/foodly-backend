//! The main entry point for the Foodly backend application.
//!
//! Initializes the server, database pool, global state, and sets up
//! the Axum router with CORS and authentication middlewares.

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

/// The shared application state injected into all Axum handlers.
///
/// Holds the database connection pool and mocked in-memory state until
/// full database integration is completed.
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    println!("Creating database connection pool...");
    let pool = db::init_pool().await?;
    println!("Database connection pool created.");

    let state = AppState { pool };

    let app = Router::new()
        .nest("/api/v1", api::rest::router())
        .route_layer(middleware::from_fn(auth_middleware))
        .layer(cors)
        .with_state(state);

    let server_address =
        std::env::var("SERVER_ADDRESS").unwrap_or_else(|_| "127.0.0.1".to_string());
    let server_port = std::env::var("SERVER_PORT").unwrap_or_else(|_| "8080".to_string());
    let bind_addr = format!("{}:{}", server_address, server_port);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    println!("Listening on {}...", bind_addr);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn auth_middleware(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    // Mock Authentication: We just require SOME Bearer token.
    // If it's there, we assign user_id = 1.
    // If not, we still assign user_id = 1 but maybe in a real app we'd return 401.
    let auth_header = req.headers().get(header::AUTHORIZATION);

    // For this mock, we will just assume user 1 is logged in if the token is somewhat valid,
    // or even unconditionally, to make testing easy.
    // Let's enforce that "Bearer" is present.
    if let Some(auth_value) = auth_header
        && let Ok(auth_str) = auth_value.to_str()
        && auth_str.starts_with("Bearer ")
    {
        // Mock user ID
        let mock_user_id = 1;
        req.extensions_mut().insert(mock_user_id);
        return Ok(next.run(req).await);
    }

    Err(StatusCode::UNAUTHORIZED)
}

//! The main entry point for the Foodly backend application.
//!
//! Initializes the server, database pool, global state, and sets up
//! the Axum router with CORS and authentication middlewares.

use foodly_backend::{AppState, app, db};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("Creating database connection pool...");
    let pool = db::init_pool().await?;
    println!("Database connection pool created.");

    let state = AppState { pool };
    let router = app(state);

    let server_address =
        std::env::var("SERVER_ADDRESS").unwrap_or_else(|_| "127.0.0.1".to_string());
    let server_port = std::env::var("SERVER_PORT").unwrap_or_else(|_| "8080".to_string());
    let bind_addr = format!("{}:{}", server_address, server_port);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    println!("Listening on {}...", bind_addr);
    axum::serve(listener, router).await?;

    Ok(())
}

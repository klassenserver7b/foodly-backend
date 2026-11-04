//! Database connection and migration management.
//!
//! Exposes functionality to connect to the PostgreSQL instance and apply
//! schema migrations automatically on startup.

use sqlx::{PgPool, postgres::PgPoolOptions};
use std::env;

/// Initializes the PostgreSQL connection pool and runs pending migrations.
///
/// Reads connection configuration from environment variables, defaults
/// to reasonable local dev values, and connects using `sqlx`.
pub async fn init_pool() -> anyhow::Result<PgPool> {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string());
        let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "password".to_string());
        let db = env::var("POSTGRES_DB").unwrap_or_else(|_| "foodly".to_string());
        let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
        format!("postgres://{}:{}@{}:{}/{}", user, password, host, port, db)
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    // Run migrations
    sqlx::migrate!("src/db/migrations").run(&pool).await?;

    Ok(pool)
}

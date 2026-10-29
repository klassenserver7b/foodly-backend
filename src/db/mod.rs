//! Database connection and migration management for diesel-async.
//!
//! Exposes functionality to connect to the PostgreSQL instance and apply
//! schema migrations automatically on startup.

use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::pooled_connection::deadpool::Pool;
use std::env;

pub type DbPool = Pool<AsyncPgConnection>;

/// Idempotent schema migrations applied in order on startup.
///
/// The files guard every statement with `IF NOT EXISTS`, so re-running is safe.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/001_create_users.sql"),
    include_str!("migrations/002_create_tags_and_ingredients.sql"),
    include_str!("migrations/003_create_images.sql"),
    include_str!("migrations/004_create_recipes.sql"),
    include_str!("migrations/005_create_sections.sql"),
    include_str!("migrations/006_create_steps.sql"),
    include_str!("migrations/007_create_recipe_ingredients.sql"),
    include_str!("migrations/008_create_permissions.sql"),
    include_str!("migrations/009_create_ratings.sql"),
    include_str!("migrations/010_create_categories.sql"),
    include_str!("migrations/011_create_groups.sql"),
    include_str!("migrations/012_create_auth_tokens.sql"),
];

/// Initializes the diesel-async connection pool and applies pending migrations.
///
/// Reads connection configuration from environment variables, defaults
/// to reasonable local dev values, and connects using `diesel-async`.
pub async fn init_pool() -> anyhow::Result<DbPool> {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string());
        let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "password".to_string());
        let db = env::var("POSTGRES_DB").unwrap_or_else(|_| "foodly".to_string());
        let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
        format!("postgres://{}:{}@{}:{}/{}", user, password, host, port, db)
    });

    let config = AsyncDieselConnectionManager::<AsyncPgConnection>::new(database_url);
    let pool = Pool::builder(config).build()?;

    run_migrations(&pool).await?;

    Ok(pool)
}

/// Applies all migrations; every statement is guarded by `IF NOT EXISTS`.
pub async fn run_migrations(pool: &DbPool) -> anyhow::Result<()> {
    use diesel_async::SimpleAsyncConnection;

    let mut conn = pool.get().await?;
    for sql in MIGRATIONS {
        conn.batch_execute(sql).await?;
    }
    Ok(())
}

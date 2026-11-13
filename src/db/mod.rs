//! Database connection and migration management for diesel-async.
//!
//! Exposes functionality to connect to the PostgreSQL instance and apply
//! schema migrations automatically on startup.

use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::pooled_connection::deadpool::Pool;
use std::env;

use diesel_async::RunQueryDsl;

pub type DbPool = Pool<AsyncPgConnection>;

/// Serializes all mutations of the same image hash for the duration of the
/// current transaction.
///
/// Every insert/delete of a file-backed `images` hash must take this lock
/// first, otherwise the database row and the file on disk can diverge (e.g.
/// a concurrent upload re-creating a row right after a delete commits but
/// before the file is removed). Released automatically on commit/rollback.
pub async fn lock_image_hash(conn: &mut AsyncPgConnection, hash: &str) -> diesel::QueryResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind::<diesel::sql_types::Text, _>(hash)
        .execute(conn)
        .await?;
    Ok(())
}

/// Idempotent schema migrations applied in order on startup.
///
/// The files guard every statement with `IF NOT EXISTS`, so re-running is safe.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/001_users_and_auth.sql"),
    include_str!("migrations/002_catalog.sql"),
    include_str!("migrations/003_images.sql"),
    include_str!("migrations/004_recipes.sql"),
    include_str!("migrations/005_user_organizations.sql"),
    include_str!("migrations/006_add_access_since.sql"),
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
        format!("postgres://{user}:{password}@{host}:{port}/{db}")
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

//! Seeds the database with mock users, recipes, and tags.
//!
//! Usage: `cargo run --bin seed`
//!
//! Seeded users get pepper-bound argon2id password hashes generated at
//! seed time (see `hash_password`), so they can actually log in with
//! `password123`.

use foodly_backend::db;
use foodly_backend::secrets::bootstrap_secrets;
use foodly_backend::services::auth::hash_password;
use foodly_backend::test_secrets::SEED_PASSWORD;

fn main() -> anyhow::Result<()> {
    // Load .env so PASSWORD_PEPPER / JWT_SECRET are available
    dotenvy::dotenv().ok();

    // Bootstrap secrets if missing (same logic as main.rs). This mutates the
    // process environment via `std::env::set_var`, which is unsafe to race with
    // other threads, so it must run before the Tokio runtime spawns workers.
    bootstrap_secrets();

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(seed())
}

async fn seed() -> anyhow::Result<()> {
    let pool = db::init_pool().await?;

    // Hash the seed password with the current pepper
    let hash = hash_password(SEED_PASSWORD)?;

    // Read seed.sql and substitute the placeholder
    let seed_sql = std::fs::read_to_string("resources/mock/seed.sql")?;
    let seed_sql = seed_sql.replace("{{PASSWORD_HASH}}", &hash);

    // Execute the seed SQL
    {
        use diesel_async::SimpleAsyncConnection;
        let mut conn = pool.get().await?;
        conn.batch_execute(&seed_sql).await?;
    }

    println!("Seeded users with password `{SEED_PASSWORD}`");
    Ok(())
}

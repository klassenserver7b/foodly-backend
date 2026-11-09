use foodly_backend::secrets::bootstrap_secrets;
use foodly_backend::services::auth::{jwt_secret, password_pepper};
use foodly_backend::{AppState, app, db};

fn main() -> anyhow::Result<()> {
    // Load .env and bootstrap secrets in a single-threaded, synchronous
    // context BEFORE the tokio runtime is created. This makes the
    // `unsafe { std::env::set_var }` in `secrets.rs` genuinely safe:
    // no other threads exist yet.
    dotenvy::dotenv().ok();
    bootstrap_secrets();

    // Eagerly validate: refuse to start with missing/short secrets
    // instead of panicking on first request.
    let _ = jwt_secret();
    let _ = password_pepper();

    // Build the multi-threaded runtime only after secrets are resolved.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let pool = db::init_pool().await?;

    let storage_path_str =
        std::env::var("IMAGE_STORAGE_PATH").unwrap_or_else(|_| "./storage/images".to_string());
    let image_storage_path = std::path::PathBuf::from(storage_path_str);

    let state = AppState {
        pool,
        image_storage_path,
    };
    let router = app(state);

    let server_address =
        std::env::var("SERVER_ADDRESS").unwrap_or_else(|_| "127.0.0.1".to_string());
    let server_port = std::env::var("SERVER_PORT").unwrap_or_else(|_| "8080".to_string());
    let bind_addr = format!("{server_address}:{server_port}");

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    println!("Listening on {bind_addr}");

    // into_make_service_with_connect_info is required for the rate
    // limiter's PeerIpKeyExtractor to extract the client IP from the
    // connection info.
    let make_service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
    axum::serve(listener, make_service).await?;

    Ok(())
}

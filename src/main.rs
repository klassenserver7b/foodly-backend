use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

pub mod api;
pub mod db;
pub mod error;
pub mod macros;
pub mod models;

use models::recipe::Recipe;

#[derive(Clone)]
pub struct AppState {
    pub recipes: Arc<RwLock<Vec<Recipe>>>,
    pub ingredients: Arc<HashMap<i32, String>>,
    pub next_recipe_id: Arc<AtomicI32>,
}

#[derive(Deserialize)]
struct IngredientMock {
    id: i32,
    name: String,
}

#[tokio::main]
async fn main() {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let addr = if let Ok(addr) = env::var("SERVER_ADDRESS") {
        addr
    } else {
        "127.0.0.1".to_string()
    };
    let port = if let Ok(port) = env::var("SERVER_PORT") {
        port
    } else {
        "8080".to_string()
    };

    let recipes_json = include_str!(mock_resource!("ingredients.json"));
    let recipes: Vec<Recipe> =
        serde_json::from_str(recipes_json).expect("Failed to parse recipes.json");
    let max_id = recipes.iter().map(|r| r.id).max().unwrap_or(0);

    let ingredients_json = include_str!(mock_resource!("ingredients.json"));
    let ingredients_mock: Vec<IngredientMock> =
        serde_json::from_str(ingredients_json).expect("Failed to parse ingredients.json");
    let mut ingredients = HashMap::new();
    for i in ingredients_mock {
        ingredients.insert(i.id, i.name);
    }

    let state = AppState {
        recipes: Arc::new(RwLock::new(recipes)),
        ingredients: Arc::new(ingredients),
        next_recipe_id: Arc::new(AtomicI32::new(max_id + 1)),
    };

    let app = Router::new()
        .nest("/api/v1", api::rest::router())
        .route_layer(middleware::from_fn(auth_middleware))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", addr, port))
        .await
        .unwrap();
    println!("Listening on port {}...", port);
    axum::serve(listener, app).await.unwrap();
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

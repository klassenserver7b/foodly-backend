use axum::Router;

pub mod recipes;

pub fn router() -> Router<crate::AppState> {
    Router::new().nest("/recipes", recipes::router())
}

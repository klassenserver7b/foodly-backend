use axum::Router;

pub mod catalogs;
pub mod recipes;

pub fn router() -> Router<crate::AppState> {
    Router::new()
        .nest("/recipes", recipes::router())
        .nest("/tags", catalogs::tags_router())
        .nest("/ingredients", catalogs::ingredients_router())
}

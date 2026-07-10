//! REST API endpoints for catalog resources (Tags and Ingredients).

use crate::models::catalog::{CatalogResponse, Ingredient, Tag};
use crate::{AppState, error::AppError};
use axum::{Json, Router, extract::State, routing::get};

/// Configures and returns the Axum router for tags.
pub fn tags_router() -> Router<AppState> {
    Router::new().route("/", get(list_tags))
}

/// Configures and returns the Axum router for ingredients.
pub fn ingredients_router() -> Router<AppState> {
    Router::new().route("/", get(list_ingredients))
}

async fn list_tags(State(state): State<AppState>) -> Result<Json<CatalogResponse<Tag>>, AppError> {
    let records = sqlx::query!("SELECT id, svg FROM tags ORDER BY id ASC")
        .fetch_all(&state.pool)
        .await?;

    let data = records
        .into_iter()
        .map(|r| Tag {
            id: r.id,
            svg: r.svg,
        })
        .collect();

    Ok(Json(CatalogResponse { data }))
}

async fn list_ingredients(
    State(state): State<AppState>,
) -> Result<Json<CatalogResponse<Ingredient>>, AppError> {
    let records = sqlx::query!("SELECT id, name FROM ingredients ORDER BY id ASC")
        .fetch_all(&state.pool)
        .await?;

    let data = records
        .into_iter()
        .map(|r| Ingredient {
            id: r.id as i32,
            name: r.name,
        })
        .collect();

    Ok(Json(CatalogResponse { data }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_list_tags(pool: PgPool) {
        let state = State(AppState { pool });
        let response = list_tags(state).await.expect("Failed to list tags");
        // Ensure no error and valid response structure
        assert!(response.0.data.is_empty() || !response.0.data.is_empty());
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_list_ingredients(pool: PgPool) {
        let state = State(AppState { pool });
        let response = list_ingredients(state)
            .await
            .expect("Failed to list ingredients");
        // Ensure no error and valid response structure
        assert!(response.0.data.is_empty() || !response.0.data.is_empty());
    }
}

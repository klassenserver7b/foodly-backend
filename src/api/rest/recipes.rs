use crate::models::recipe::{
    CreateRecipe, IngredientRef, Recipe, RecipeIngredient, RecipePreview, Section,
};
use crate::{AppState, error::AppError};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_recipes).post(create_recipe))
        .route(
            "/{id}",
            get(get_recipe).put(update_recipe).delete(delete_recipe),
        )
        .route("/{id}/copy", post(copy_recipe))
}

#[derive(Deserialize)]
pub struct PaginationQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub cursor: Option<String>,
}

async fn list_recipes(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<RecipePreview>>, AppError> {
    let limit = query.limit.unwrap_or(20).min(100);
    let recipes = state.recipes.read().await;

    // Filter recipes accessible to user
    let mut accessible: Vec<RecipePreview> = recipes
        .iter()
        .filter(|r| {
            r.owner == user_id || r.editors.contains(&user_id) || r.viewers.contains(&user_id)
        })
        .cloned()
        .map(RecipePreview::from)
        .collect();

    accessible.sort_by_key(|r| r.id);

    // Simple pagination: if cursor is present, we expect it to be a string like "eyJpZCI6NDJ9".
    // For this mock, let's just parse the cursor as stringified ID.
    // Usually it is base64 encoded JSON. We'll skip proper cursor implementation for now.
    let _start_index = 0; // In a real app we'd parse the cursor
    let data: Vec<RecipePreview> = accessible.into_iter().take(limit).collect();

    Ok(Json(PaginatedResponse {
        data,
        cursor: None, // No more pages for mock
    }))
}

async fn get_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
) -> Result<Json<Recipe>, AppError> {
    let recipes = state.recipes.read().await;
    let recipe = recipes
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    if recipe.owner != user_id
        && !recipe.editors.contains(&user_id)
        && !recipe.viewers.contains(&user_id)
    {
        return Err(AppError::Forbidden);
    }

    Ok(Json(recipe.clone()))
}

fn map_create_to_recipe(state: &AppState, id: i32, owner: i32, input: CreateRecipe) -> Recipe {
    let mut next_section_id = 1;
    let mut next_ingredient_id = 1;

    let sections = input
        .sections
        .into_iter()
        .map(|s| {
            let sec_id = next_section_id;
            next_section_id += 1;

            let ingredients = s
                .ingredients
                .into_iter()
                .map(|i| {
                    let ing_id = next_ingredient_id;
                    next_ingredient_id += 1;

                    let ingredient_ref = i.ingredient.and_then(|id| {
                        state.ingredients.get(&id).map(|name| IngredientRef {
                            id,
                            name: name.clone(),
                        })
                    });

                    RecipeIngredient {
                        id: ing_id,
                        ingredient: ingredient_ref,
                        text: i.text,
                        amount: i.amount,
                        amount_prefix: i.amount_prefix,
                        unit: i.unit,
                    }
                })
                .collect();

            Section {
                id: sec_id,
                name: s.name,
                ingredients,
                steps: s.steps,
            }
        })
        .collect();

    let now = Utc::now().to_rfc3339();

    Recipe {
        id,
        owner,
        editors: vec![],
        viewers: vec![],
        name: input.name,
        tags: input.tags,
        source: input.source,
        rating: vec![],
        time: input.time,
        work_minutes: input.work_minutes,
        overall_minutes: input.overall_minutes,
        size_number: input.size_number,
        size_text: input.size_text,
        notes: input.notes,
        main_image: input.main_image,
        images: input.images,
        sections,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    }
}

async fn create_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Json(payload): Json<CreateRecipe>,
) -> Result<Json<Recipe>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }

    let id = state.next_recipe_id.fetch_add(1, Ordering::SeqCst);
    let recipe = map_create_to_recipe(&state, id, user_id, payload);

    state.recipes.write().await.push(recipe.clone());

    Ok(Json(recipe))
}

async fn update_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
    Json(payload): Json<CreateRecipe>,
) -> Result<Json<Recipe>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }

    let mut recipes = state.recipes.write().await;
    let index = recipes
        .iter()
        .position(|r| r.id == id)
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    let existing = &recipes[index];
    if existing.owner != user_id && !existing.editors.contains(&user_id) {
        return Err(AppError::Forbidden);
    }

    let mut updated_recipe = map_create_to_recipe(&state, id, existing.owner, payload);
    updated_recipe.editors = existing.editors.clone();
    updated_recipe.viewers = existing.viewers.clone();
    updated_recipe.rating = existing.rating.clone();
    updated_recipe.created_at = existing.created_at.clone();
    updated_recipe.updated_at = Some(Utc::now().to_rfc3339());

    recipes[index] = updated_recipe.clone();

    Ok(Json(updated_recipe))
}

async fn delete_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
) -> Result<axum::http::StatusCode, AppError> {
    let mut recipes = state.recipes.write().await;
    let index = recipes
        .iter()
        .position(|r| r.id == id)
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    if recipes[index].owner != user_id {
        return Err(AppError::Forbidden);
    }

    recipes.remove(index);

    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn copy_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
) -> Result<Json<Recipe>, AppError> {
    let mut recipes = state.recipes.write().await;
    let index = recipes
        .iter()
        .position(|r| r.id == id)
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    let existing = &recipes[index];
    if existing.owner != user_id
        && !existing.editors.contains(&user_id)
        && !existing.viewers.contains(&user_id)
    {
        return Err(AppError::Forbidden);
    }

    let mut copied_recipe = existing.clone();
    let new_id = state.next_recipe_id.fetch_add(1, Ordering::SeqCst);
    copied_recipe.id = new_id;
    copied_recipe.owner = user_id;
    copied_recipe.editors = vec![];
    copied_recipe.viewers = vec![];
    copied_recipe.name = format!("{} (Copy)", copied_recipe.name);
    let now = Utc::now().to_rfc3339();
    copied_recipe.created_at = Some(now.clone());
    copied_recipe.updated_at = Some(now);

    recipes.push(copied_recipe.clone());

    Ok(Json(copied_recipe))
}

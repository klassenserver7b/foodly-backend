//! REST API endpoints for recipe management.
//!
//! Provides handlers for listing, creating, reading, updating, deleting,
//! and copying recipes using PostgreSQL via `sqlx`.
mod helpers;

use crate::models::recipe::{CreateRecipe, Recipe, RecipePreview};
use crate::{AppState, error::AppError};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

/// Configures and returns the Axum router for recipe endpoints.
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
    let limit = query.limit.unwrap_or(20).min(100) as i64;
    let u_id = user_id as i64;

    let cursor_id: Option<i64> = match &query.cursor {
        Some(c) => Some(
            c.parse()
                .map_err(|_| AppError::Unprocessable("Invalid cursor".into()))?,
        ),
        None => None,
    };

    let mut records = sqlx::query!(
        r#"
        SELECT
            r.id, r.owner_id, r.name, r.source, r.time_display,
            r.work_minutes, r.overall_minutes, r.size_number, r.size_text, r.main_image_id,
            r.created_at, r.updated_at,
            COALESCE((SELECT array_agg(user_id) FROM recipe_editors WHERE recipe_id = r.id), '{}') as "editors!",
            COALESCE((SELECT array_agg(user_id) FROM recipe_viewers WHERE recipe_id = r.id), '{}') as "viewers!",
            COALESCE((SELECT array_agg(tag_id ORDER BY position) FROM recipe_tags WHERE recipe_id = r.id), '{}') as "tags!"
        FROM recipes r
        WHERE (r.owner_id = $1
           OR EXISTS (SELECT 1 FROM recipe_editors e WHERE e.recipe_id = r.id AND e.user_id = $1)
           OR EXISTS (SELECT 1 FROM recipe_viewers v WHERE v.recipe_id = r.id AND v.user_id = $1))
          AND ($3::bigint IS NULL OR r.id > $3)
        ORDER BY r.id ASC
        LIMIT $2
        "#,
        u_id, limit + 1, cursor_id
    ).fetch_all(&state.pool).await?;

    let has_more = records.len() > limit as usize;
    if has_more {
        records.pop();
    }

    let mut next_cursor = None;
    if let Some(last) = records.last() {
        next_cursor = Some(last.id.to_string());
    }

    let mut data = Vec::with_capacity(records.len());
    for rec in records {
        data.push(RecipePreview {
            id: rec.id as i32,
            owner: rec.owner_id as i32,
            editors: rec.editors.into_iter().map(|v| v as i32).collect(),
            viewers: rec.viewers.into_iter().map(|v| v as i32).collect(),
            name: rec.name,
            tags: rec.tags,
            source: rec.source,
            rating: vec![],
            time: rec.time_display,
            work_minutes: rec.work_minutes,
            overall_minutes: rec.overall_minutes,
            size_number: rec.size_number,
            size_text: rec.size_text,
            main_image: rec.main_image_id.map(|id| id as i32),
            created_at: Some(rec.created_at.to_rfc3339()),
            updated_at: Some(rec.updated_at.to_rfc3339()),
        });
    }

    Ok(Json(PaginatedResponse {
        data,
        cursor: if has_more { next_cursor } else { None },
    }))
}

async fn get_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
) -> Result<Json<Recipe>, AppError> {
    let r_id = id as i64;
    let u_id = user_id as i64;

    let rec = sqlx::query!(
        r#"
        SELECT
            r.id, r.owner_id, r.name, r.source, r.time_display,
            r.work_minutes, r.overall_minutes, r.size_number, r.size_text, r.main_image_id, r.notes,
            r.created_at, r.updated_at,
            COALESCE((SELECT array_agg(user_id) FROM recipe_editors WHERE recipe_id = r.id), '{}') as "editors!",
            COALESCE((SELECT array_agg(user_id) FROM recipe_viewers WHERE recipe_id = r.id), '{}') as "viewers!",
            COALESCE((SELECT array_agg(tag_id ORDER BY position) FROM recipe_tags WHERE recipe_id = r.id), '{}') as "tags!",
            COALESCE((SELECT array_agg(image_id ORDER BY position) FROM recipe_images WHERE recipe_id = r.id), '{}') as "images!"
        FROM recipes r
        WHERE r.id = $1
        "#,
        r_id
    ).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    if rec.owner_id != u_id && !rec.editors.contains(&u_id) && !rec.viewers.contains(&u_id) {
        return Err(AppError::Forbidden);
    }

    let sections = helpers::fetch_sections(r_id, &state.pool).await?;

    let recipe = Recipe {
        id: rec.id as i32,
        owner: rec.owner_id as i32,
        editors: rec.editors.into_iter().map(|v| v as i32).collect(),
        viewers: rec.viewers.into_iter().map(|v| v as i32).collect(),
        name: rec.name,
        tags: rec.tags,
        source: rec.source,
        rating: vec![],
        time: rec.time_display,
        work_minutes: rec.work_minutes,
        overall_minutes: rec.overall_minutes,
        size_number: rec.size_number,
        size_text: rec.size_text,
        notes: rec.notes,
        main_image: rec.main_image_id.map(|id| id as i32),
        images: rec.images.into_iter().map(|v| v as i32).collect(),
        sections,
        created_at: Some(rec.created_at.to_rfc3339()),
        updated_at: Some(rec.updated_at.to_rfc3339()),
    };

    Ok(Json(recipe))
}

async fn create_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Json(payload): Json<CreateRecipe>,
) -> Result<Json<Recipe>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }

    let mut tx = state.pool.begin().await?;
    let u_id = user_id as i64;
    let rec_id = sqlx::query!(
        r#"
        INSERT INTO recipes (owner_id, name, source, time_display, work_minutes, overall_minutes, size_number, size_text, notes, main_image_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id
        "#,
        u_id, payload.name, payload.source, payload.time, payload.work_minutes, payload.overall_minutes,
        payload.size_number, payload.size_text, &payload.notes, payload.main_image.map(|i| i as i64)
    ).fetch_one(&mut *tx).await?.id;

    helpers::insert_recipe_tags(rec_id, payload.tags, &mut tx).await?;
    helpers::insert_recipe_images(rec_id, payload.images, &mut tx).await?;

    for (s_pos, sec) in payload.sections.into_iter().enumerate() {
        helpers::insert_section(sec, rec_id, s_pos, &mut tx).await?;
    }

    tx.commit().await?;

    // Reuse get_recipe logic internally
    get_recipe(State(state), Extension(user_id), Path(rec_id as i32)).await
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

    let r_id = id as i64;
    let u_id = user_id as i64;

    let mut tx = state.pool.begin().await?;

    helpers::check_can_edit(r_id, u_id, &mut tx).await?;
    helpers::update_recipe_metadata(r_id, &payload, &mut tx).await?;
    helpers::replace_recipe_tags(r_id, payload.tags, &mut tx).await?;
    helpers::replace_recipe_images(r_id, payload.images, &mut tx).await?;
    helpers::replace_recipe_sections(r_id, payload.sections, &mut tx).await?;

    tx.commit().await?;

    get_recipe(State(state), Extension(user_id), Path(id)).await
}

async fn delete_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
) -> Result<axum::http::StatusCode, AppError> {
    let r_id = id as i64;
    let u_id = user_id as i64;

    let res = sqlx::query!(
        "DELETE FROM recipes WHERE id = $1 AND owner_id = $2",
        r_id,
        u_id
    )
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        // Either not found or forbidden
        let exists = sqlx::query!("SELECT 1 as x FROM recipes WHERE id = $1", r_id)
            .fetch_optional(&state.pool)
            .await?
            .is_some();
        if exists {
            return Err(AppError::Forbidden);
        }
        return Err(AppError::NotFound("Recipe not found".into()));
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn copy_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
) -> Result<Json<Recipe>, AppError> {
    let original = get_recipe(State(state.clone()), Extension(user_id), Path(id))
        .await?
        .0;

    let create_payload = helpers::create_recipe_copy(original);

    create_recipe(State(state), Extension(user_id), Json(create_payload)).await
}

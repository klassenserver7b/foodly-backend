//! REST API endpoints for recipe management.
//!
//! Provides handlers for listing, creating, reading, updating, deleting,
//! and copying recipes using PostgreSQL via `sqlx`.

use crate::models::recipe::{
    CreateRecipe, IngredientRef, Recipe, RecipeIngredient, RecipePreview, Section,
};
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

    let records = sqlx::query!(
        r#"
        SELECT
            r.id, r.owner_id, r.name, r.source, r.time_display,
            r.work_minutes, r.overall_minutes, r.size_number, r.size_text, r.main_image_id,
            r.created_at, r.updated_at,
            COALESCE((SELECT array_agg(user_id) FROM recipe_editors WHERE recipe_id = r.id), '{}') as "editors!",
            COALESCE((SELECT array_agg(user_id) FROM recipe_viewers WHERE recipe_id = r.id), '{}') as "viewers!",
            COALESCE((SELECT array_agg(tag_id ORDER BY position) FROM recipe_tags WHERE recipe_id = r.id), '{}') as "tags!"
        FROM recipes r
        WHERE r.owner_id = $1
           OR EXISTS (SELECT 1 FROM recipe_editors e WHERE e.recipe_id = r.id AND e.user_id = $1)
           OR EXISTS (SELECT 1 FROM recipe_viewers v WHERE v.recipe_id = r.id AND v.user_id = $1)
        ORDER BY r.id ASC
        LIMIT $2
        "#,
        u_id, limit
    ).fetch_all(&state.pool).await?;

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

    Ok(Json(PaginatedResponse { data, cursor: None }))
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

    let sec_rows = sqlx::query!(
        "SELECT id, name FROM sections WHERE recipe_id = $1 ORDER BY position ASC",
        r_id
    )
    .fetch_all(&state.pool)
    .await?;

    let mut sections = Vec::with_capacity(sec_rows.len());
    for sec in sec_rows {
        let step_rows = sqlx::query!(
            "SELECT text FROM steps WHERE section_id = $1 ORDER BY position ASC",
            sec.id
        )
        .fetch_all(&state.pool)
        .await?;

        let ing_rows = sqlx::query!(
            r#"
            SELECT ri.id, ri.ingredient_id, i.name as "ingredient_name?", ri.text, ri.amount, ri.amount_prefix, ri.unit
            FROM recipe_ingredients ri
            LEFT JOIN ingredients i ON ri.ingredient_id = i.id
            WHERE ri.section_id = $1
            ORDER BY ri.position ASC
            "#,
            sec.id
        ).fetch_all(&state.pool).await?;

        sections.push(Section {
            id: sec.id as i32,
            name: sec.name,
            steps: step_rows.into_iter().map(|s| s.text).collect(),
            ingredients: ing_rows
                .into_iter()
                .map(|i| RecipeIngredient {
                    id: i.id as i32,
                    ingredient: i.ingredient_id.map(|id| IngredientRef {
                        id: id as i32,
                        name: i.ingredient_name.unwrap_or_default(),
                    }),
                    text: i.text,
                    amount: i.amount,
                    amount_prefix: i.amount_prefix,
                    unit: i.unit,
                })
                .collect(),
        });
    }

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

    for (pos, tag) in payload.tags.into_iter().enumerate() {
        sqlx::query!(
            "INSERT INTO recipe_tags (recipe_id, tag_id, position) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            rec_id, tag, pos as i32
        ).execute(&mut *tx).await?;
    }

    for (pos, img) in payload.images.into_iter().enumerate() {
        sqlx::query!(
            "INSERT INTO recipe_images (recipe_id, image_id, position) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            rec_id, img as i64, pos as i32
        ).execute(&mut *tx).await?;
    }

    for (s_pos, sec) in payload.sections.into_iter().enumerate() {
        let sec_id = sqlx::query!(
            "INSERT INTO sections (recipe_id, name, position) VALUES ($1, $2, $3) RETURNING id",
            rec_id,
            sec.name,
            s_pos as i32
        )
        .fetch_one(&mut *tx)
        .await?
        .id;

        for (st_pos, step) in sec.steps.into_iter().enumerate() {
            sqlx::query!(
                "INSERT INTO steps (section_id, text, position) VALUES ($1, $2, $3)",
                sec_id,
                step,
                st_pos as i32
            )
            .execute(&mut *tx)
            .await?;
        }

        for (i_pos, ing) in sec.ingredients.into_iter().enumerate() {
            sqlx::query!(
                "INSERT INTO recipe_ingredients (section_id, ingredient_id, text, amount, amount_prefix, unit, position) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                sec_id, ing.ingredient.map(|i| i as i64), ing.text, ing.amount, ing.amount_prefix, ing.unit, i_pos as i32
            ).execute(&mut *tx).await?;
        }
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

    // Check permissions
    let owner_check = sqlx::query!(
        "SELECT owner_id, EXISTS(SELECT 1 FROM recipe_editors WHERE recipe_id = $1 AND user_id = $2) as is_editor FROM recipes WHERE id = $1",
        r_id, u_id
    ).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    if owner_check.owner_id != u_id && !owner_check.is_editor.unwrap_or(false) {
        return Err(AppError::Forbidden);
    }

    // Update main recipe
    sqlx::query!(
        r#"
        UPDATE recipes SET
            name = $2, source = $3, time_display = $4, work_minutes = $5, overall_minutes = $6,
            size_number = $7, size_text = $8, notes = $9, main_image_id = $10, updated_at = now()
        WHERE id = $1
        "#,
        r_id,
        payload.name,
        payload.source,
        payload.time,
        payload.work_minutes,
        payload.overall_minutes,
        payload.size_number,
        payload.size_text,
        &payload.notes,
        payload.main_image.map(|i| i as i64)
    )
    .execute(&mut *tx)
    .await?;

    // Recreate tags, images, sections
    sqlx::query!("DELETE FROM recipe_tags WHERE recipe_id = $1", r_id)
        .execute(&mut *tx)
        .await?;
    for (pos, tag) in payload.tags.into_iter().enumerate() {
        sqlx::query!("INSERT INTO recipe_tags (recipe_id, tag_id, position) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING", r_id, tag, pos as i32).execute(&mut *tx).await?;
    }

    sqlx::query!("DELETE FROM recipe_images WHERE recipe_id = $1", r_id)
        .execute(&mut *tx)
        .await?;
    for (pos, img) in payload.images.into_iter().enumerate() {
        sqlx::query!("INSERT INTO recipe_images (recipe_id, image_id, position) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING", r_id, img as i64, pos as i32).execute(&mut *tx).await?;
    }

    sqlx::query!("DELETE FROM sections WHERE recipe_id = $1", r_id)
        .execute(&mut *tx)
        .await?;
    for (s_pos, sec) in payload.sections.into_iter().enumerate() {
        let sec_id = sqlx::query!(
            "INSERT INTO sections (recipe_id, name, position) VALUES ($1, $2, $3) RETURNING id",
            r_id,
            sec.name,
            s_pos as i32
        )
        .fetch_one(&mut *tx)
        .await?
        .id;
        for (st_pos, step) in sec.steps.into_iter().enumerate() {
            sqlx::query!(
                "INSERT INTO steps (section_id, text, position) VALUES ($1, $2, $3)",
                sec_id,
                step,
                st_pos as i32
            )
            .execute(&mut *tx)
            .await?;
        }
        for (i_pos, ing) in sec.ingredients.into_iter().enumerate() {
            sqlx::query!("INSERT INTO recipe_ingredients (section_id, ingredient_id, text, amount, amount_prefix, unit, position) VALUES ($1, $2, $3, $4, $5, $6, $7)", sec_id, ing.ingredient.map(|i| i as i64), ing.text, ing.amount, ing.amount_prefix, ing.unit, i_pos as i32).execute(&mut *tx).await?;
        }
    }

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

    let create_payload = CreateRecipe {
        name: format!("{} (Copy)", original.name),
        tags: original.tags,
        source: original.source,
        time: original.time,
        work_minutes: original.work_minutes,
        overall_minutes: original.overall_minutes,
        size_number: original.size_number,
        size_text: original.size_text,
        notes: original.notes,
        main_image: original.main_image,
        images: original.images,
        sections: original
            .sections
            .into_iter()
            .map(|s| crate::models::recipe::CreateSection {
                name: s.name,
                ingredients: s
                    .ingredients
                    .into_iter()
                    .map(|i| crate::models::recipe::CreateRecipeIngredient {
                        ingredient: i.ingredient.map(|ing| ing.id),
                        text: i.text,
                        amount: i.amount,
                        amount_prefix: i.amount_prefix,
                        unit: i.unit,
                    })
                    .collect(),
                steps: s.steps,
            })
            .collect(),
    };

    create_recipe(State(state), Extension(user_id), Json(create_payload)).await
}

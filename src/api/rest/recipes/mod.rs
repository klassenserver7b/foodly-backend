//! REST API endpoints for recipe management.
//!
//! Provides handlers for listing, creating, reading, updating, deleting,
//! and copying recipes using PostgreSQL via `sqlx`.
mod helpers;

use crate::models::recipe::{
    CreateRecipe, Order, Recipe, RecipeAccessRights, RecipePreview, RecipeSearchQuery,
    RecipeShareState,
};
use crate::{AppState, error::AppError};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

/// Configures and returns the Axum router for recipe endpoints.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_recipes).post(create_recipe))
        .route("/search", post(search_recipes))
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

async fn search_recipes(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    query: Option<Json<RecipeSearchQuery>>,
) -> Result<Json<PaginatedResponse<RecipePreview>>, AppError> {
    let query = query.map(|q| q.0).unwrap_or_default();
    let u_id = user_id as i64;
    let limit = query.limit.unwrap_or(20).min(100);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;

    let mut qb = sqlx::QueryBuilder::new(
        r#"
        SELECT
            r.id, r.owner_id, r.name, r.source, r.time_display,
            r.work_minutes, r.overall_minutes, r.size_number, r.size_text, r.main_image_id,
            r.created_at, r.updated_at,
            COALESCE((SELECT array_agg(user_id) FROM recipe_editors WHERE recipe_id = r.id), '{}') as editors,
            COALESCE((SELECT array_agg(user_id) FROM recipe_viewers WHERE recipe_id = r.id), '{}') as viewers,
            COALESCE((SELECT array_agg(tag_id ORDER BY position) FROM recipe_tags WHERE recipe_id = r.id), '{}') as tags,
            COALESCE((SELECT AVG(rating) FROM user_ratings WHERE recipe_id = r.id), 0.0) as avg_rating
        FROM recipes r
        WHERE 1=1
        "#,
    );

    let has_access = query
        .filters
        .as_ref()
        .and_then(|f| f.access_rights.as_ref())
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let has_shares = query
        .filters
        .as_ref()
        .and_then(|f| f.share_states.as_ref())
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    if has_access || has_shares {
        qb.push(" AND (1=0");

        if let Some(accs) = query.filters.as_ref().unwrap().access_rights.as_ref() {
            for a in accs {
                match a {
                    RecipeAccessRights::Owner => {
                        qb.push(" OR r.owner_id = ");
                        qb.push_bind(u_id);
                    }
                    RecipeAccessRights::Editor => {
                        qb.push(" OR EXISTS (SELECT 1 FROM recipe_editors e WHERE e.recipe_id = r.id AND e.user_id = ");
                        qb.push_bind(u_id);
                        qb.push(")");
                    }
                    RecipeAccessRights::Viewer => {
                        qb.push(" OR EXISTS (SELECT 1 FROM recipe_viewers v WHERE v.recipe_id = r.id AND v.user_id = ");
                        qb.push_bind(u_id);
                        qb.push(")");
                    }
                }
            }
        }

        if let Some(shs) = query.filters.as_ref().unwrap().share_states.as_ref() {
            for s in shs {
                match s {
                    RecipeShareState::Private => {
                        qb.push(" OR (r.owner_id = ");
                        qb.push_bind(u_id);
                        qb.push(" AND NOT EXISTS (SELECT 1 FROM recipe_editors WHERE recipe_id = r.id) AND NOT EXISTS (SELECT 1 FROM recipe_viewers WHERE recipe_id = r.id))");
                    }
                    RecipeShareState::Shared => {
                        qb.push(" OR EXISTS (SELECT 1 FROM recipe_viewers WHERE recipe_id = r.id)");
                    }
                    RecipeShareState::Collaborative => {
                        qb.push(" OR EXISTS (SELECT 1 FROM recipe_editors WHERE recipe_id = r.id)");
                    }
                }
            }
        }

        qb.push(")");

        qb.push(" AND (r.owner_id = ");
        qb.push_bind(u_id);
        qb.push(
            " OR EXISTS (SELECT 1 FROM recipe_editors e WHERE e.recipe_id = r.id AND e.user_id = ",
        );
        qb.push_bind(u_id);
        qb.push(")");
        qb.push(
            " OR EXISTS (SELECT 1 FROM recipe_viewers v WHERE v.recipe_id = r.id AND v.user_id = ",
        );
        qb.push_bind(u_id);
        qb.push("))");
    } else {
        qb.push(" AND (r.owner_id = ");
        qb.push_bind(u_id);
        qb.push(
            " OR EXISTS (SELECT 1 FROM recipe_editors e WHERE e.recipe_id = r.id AND e.user_id = ",
        );
        qb.push_bind(u_id);
        qb.push(")");
        qb.push(
            " OR EXISTS (SELECT 1 FROM recipe_viewers v WHERE v.recipe_id = r.id AND v.user_id = ",
        );
        qb.push_bind(u_id);
        qb.push("))");
    }

    if let Some(f) = &query.filters {
        if let Some(cats) = &f.categories
            && !cats.is_empty()
        {
            qb.push(" AND (SELECT count(DISTINCT ucr.category_id) FROM user_category_recipes ucr WHERE ucr.recipe_id = r.id AND ucr.category_id = ANY(");
            qb.push_bind(cats);
            qb.push(")) = ");
            qb.push(cats.len().to_string());
        }
        if let Some(tags) = &f.tags
            && !tags.is_empty()
        {
            qb.push(" AND (SELECT count(DISTINCT rt.tag_id) FROM recipe_tags rt WHERE rt.recipe_id = r.id AND rt.tag_id = ANY(");
            qb.push_bind(tags);
            qb.push(")) = ");
            qb.push(tags.len().to_string());
        }
        if let Some(ings) = &f.ingredients
            && !ings.is_empty()
        {
            qb.push(" AND (SELECT count(DISTINCT ri.ingredient_id) FROM sections s JOIN recipe_ingredients ri ON s.id = ri.section_id WHERE s.recipe_id = r.id AND ri.ingredient_id = ANY(");
            qb.push_bind(ings);
            qb.push(")) = ");
            qb.push(ings.len().to_string());
        }
        if let Some(max_time) = f.max_work_time {
            qb.push(" AND r.work_minutes <= ");
            qb.push_bind(max_time);
        }
    }

    if let Some(s) = &query.sort {
        let order = if s.order == Order::Desc {
            "DESC NULLS LAST"
        } else {
            "ASC NULLS LAST"
        };
        match s.field.as_str() {
            "name" => {
                qb.push(" ORDER BY r.name ");
                qb.push(order);
            }
            "worktime" | "work_time" => {
                qb.push(" ORDER BY r.work_minutes ");
                qb.push(order);
            }
            "totaltime" | "total_time" => {
                qb.push(" ORDER BY r.overall_minutes ");
                qb.push(order);
            }
            "rating" => {
                qb.push(" ORDER BY avg_rating ");
                qb.push(order);
            }
            _ => {
                qb.push(" ORDER BY r.id ASC");
            }
        }
    } else {
        qb.push(" ORDER BY r.id ASC");
    }

    qb.push(" LIMIT ");
    qb.push_bind(limit + 1);
    qb.push(" OFFSET ");
    qb.push_bind(offset);

    let mut records = qb.build().fetch_all(&state.pool).await?;

    let has_more = records.len() > limit as usize;
    if has_more {
        records.pop();
    }

    let mut data = Vec::with_capacity(records.len());
    for rec in records {
        let id: i64 = rec.try_get("id")?;
        let owner: i64 = rec.try_get("owner_id")?;
        let name: String = rec.try_get("name")?;
        let source: Option<String> = rec.try_get("source")?;
        let time_display: Option<String> = rec.try_get("time_display")?;
        let work_minutes: Option<i32> = rec.try_get("work_minutes")?;
        let overall_minutes: Option<i32> = rec.try_get("overall_minutes")?;
        let size_number: Option<i32> = rec.try_get("size_number")?;
        let size_text: Option<String> = rec.try_get("size_text")?;
        let main_image_id: Option<i64> = rec.try_get("main_image_id")?;

        let created_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc> =
            rec.try_get("created_at")?;
        let updated_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc> =
            rec.try_get("updated_at")?;

        let editors: Vec<i64> = rec.try_get("editors")?;
        let viewers: Vec<i64> = rec.try_get("viewers")?;
        let tags: Vec<String> = rec.try_get("tags")?;

        data.push(RecipePreview {
            id: id as i32,
            owner: owner as i32,
            editors: editors.into_iter().map(|v| v as i32).collect(),
            viewers: viewers.into_iter().map(|v| v as i32).collect(),
            name,
            tags,
            source,
            rating: vec![],
            time: time_display,
            work_minutes,
            overall_minutes,
            size_number,
            size_text,
            main_image: main_image_id.map(|id| id as i32),
            created_at: Some(created_at.to_rfc3339()),
            updated_at: Some(updated_at.to_rfc3339()),
        });
    }

    let cursor = if has_more {
        Some((page + 1).to_string())
    } else {
        None
    };

    Ok(Json(PaginatedResponse { data, cursor }))
}

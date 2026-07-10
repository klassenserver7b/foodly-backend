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
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub cursor: Option<String>,
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

async fn fetch_recipes(
    state: &AppState,
    user_id: i32,
    pagination: PaginationQuery,
    query: RecipeSearchQuery,
) -> Result<PaginatedResponse<RecipePreview>, AppError> {
    let u_id = user_id as i64;
    let limit = pagination.limit.unwrap_or(20).min(100);
    let page = pagination.page.unwrap_or(1).max(1);
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

        if let Some(filters) = &query.filters {
            if let Some(accs) = filters.access_rights.as_ref() {
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

            if let Some(shs) = filters.share_states.as_ref() {
                for s in shs {
                    match s {
                        RecipeShareState::Private => {
                            qb.push(" OR (r.owner_id = ");
                            qb.push_bind(u_id);
                            qb.push(" AND NOT EXISTS (SELECT 1 FROM recipe_editors WHERE recipe_id = r.id) AND NOT EXISTS (SELECT 1 FROM recipe_viewers WHERE recipe_id = r.id))");
                        }
                        RecipeShareState::Shared => {
                            qb.push(
                                " OR EXISTS (SELECT 1 FROM recipe_viewers WHERE recipe_id = r.id)",
                            );
                        }
                        RecipeShareState::Collaborative => {
                            qb.push(
                                " OR EXISTS (SELECT 1 FROM recipe_editors WHERE recipe_id = r.id)",
                            );
                        }
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

    Ok(PaginatedResponse { data, cursor })
}

async fn list_recipes(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Query(pagination): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<RecipePreview>>, AppError> {
    let search_query = RecipeSearchQuery::default();
    let res = fetch_recipes(&state, user_id, pagination, search_query).await?;
    Ok(Json(res))
}

async fn search_recipes(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Query(pagination): Query<PaginationQuery>,
    query: Option<Json<RecipeSearchQuery>>,
) -> Result<Json<PaginatedResponse<RecipePreview>>, AppError> {
    let query = query.map(|q| q.0).unwrap_or_default();
    let res = fetch_recipes(&state, user_id, pagination, query).await?;
    Ok(Json(res))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::recipe::CreateRecipe;
    use axum::extract::{Path, Query, State};
    use axum::{Extension, Json};
    use sqlx::PgPool;

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_recipe_lifecycle(pool: PgPool) {
        let state = State(AppState { pool: pool.clone() });

        let user_id_i64 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('test', 'test@example.com', 'hash') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap()
            .id;

        let user_id = Extension(user_id_i64 as i32);

        // 1. Create a recipe
        let create_payload = CreateRecipe {
            name: "Test Recipe".to_string(),
            tags: vec![],
            source: None,
            time: None,
            work_minutes: Some(15),
            overall_minutes: Some(30),
            size_number: Some(2),
            size_text: Some("portions".to_string()),
            notes: vec![],
            main_image: None,
            images: vec![],
            sections: vec![],
        };

        let created_recipe = create_recipe(state.clone(), user_id, Json(create_payload))
            .await
            .expect("Failed to create recipe")
            .0;

        assert_eq!(created_recipe.name, "Test Recipe");
        assert_eq!(created_recipe.owner, user_id_i64 as i32);
        let r_id = created_recipe.id;

        // 2. Get the recipe
        let fetched = get_recipe(state.clone(), user_id, Path(r_id))
            .await
            .expect("Failed to get recipe")
            .0;
        assert_eq!(fetched.id, r_id);
        assert_eq!(fetched.name, "Test Recipe");

        // 3. Update the recipe
        let update_payload = CreateRecipe {
            name: "Updated Recipe".to_string(),
            tags: vec![],
            source: None,
            time: None,
            work_minutes: Some(20),
            overall_minutes: Some(40),
            size_number: Some(4),
            size_text: Some("portions".to_string()),
            notes: vec![],
            main_image: None,
            images: vec![],
            sections: vec![],
        };
        let updated = update_recipe(state.clone(), user_id, Path(r_id), Json(update_payload))
            .await
            .expect("Failed to update recipe")
            .0;
        assert_eq!(updated.name, "Updated Recipe");

        // 4. List recipes
        let list = list_recipes(
            state.clone(),
            user_id,
            Query(PaginationQuery {
                page: None,
                limit: None,
            }),
        )
        .await
        .expect("Failed to list recipes")
        .0;
        assert!(!list.data.is_empty());

        // 5. Delete the recipe
        let _ = delete_recipe(state.clone(), user_id, Path(r_id))
            .await
            .expect("Failed to delete recipe");

        // 6. Verify deletion
        let err = get_recipe(state.clone(), user_id, Path(r_id)).await;
        assert!(err.is_err());
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_recipe_pagination(pool: PgPool) {
        let state = State(AppState { pool: pool.clone() });

        let user_id_i64 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('test_pag', 'pag@example.com', 'hash') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap()
            .id;
        let user_id = Extension(user_id_i64 as i32);

        // Insert 25 recipes
        for i in 1..=25 {
            let create_payload = CreateRecipe {
                name: format!("Recipe {}", i),
                tags: vec![],
                source: None,
                time: None,
                work_minutes: Some(15),
                overall_minutes: Some(30),
                size_number: Some(2),
                size_text: Some("portions".to_string()),
                notes: vec![],
                main_image: None,
                images: vec![],
                sections: vec![],
            };
            let _ = create_recipe(state.clone(), user_id, Json(create_payload))
                .await
                .expect("Failed to create recipe");
        }

        // Test page 1, limit 10
        let p1 = list_recipes(
            state.clone(),
            user_id,
            Query(PaginationQuery {
                page: Some(1),
                limit: Some(10),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(p1.data.len(), 10);
        assert_eq!(p1.cursor, Some("2".to_string()));

        // Test page 2, limit 10
        let p2 = list_recipes(
            state.clone(),
            user_id,
            Query(PaginationQuery {
                page: Some(2),
                limit: Some(10),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(p2.data.len(), 10);
        assert_eq!(p2.cursor, Some("3".to_string()));

        // Test page 3, limit 10 (should only have 5)
        let p3 = list_recipes(
            state.clone(),
            user_id,
            Query(PaginationQuery {
                page: Some(3),
                limit: Some(10),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(p3.data.len(), 5);
        assert_eq!(p3.cursor, None);
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_recipe_permissions(pool: PgPool) {
        let state = State(AppState { pool: pool.clone() });

        let u1_id = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('user1', 'u1@example.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id;
        let u2_id = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('user2', 'u2@example.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id;

        let user1 = Extension(u1_id as i32);
        let user2 = Extension(u2_id as i32);

        // User 1 creates a recipe
        let payload = CreateRecipe {
            name: "User1 Recipe".to_string(),
            tags: vec![],
            source: None,
            time: None,
            work_minutes: None,
            overall_minutes: None,
            size_number: None,
            size_text: None,
            notes: vec![],
            main_image: None,
            images: vec![],
            sections: vec![],
        };
        let r1 = create_recipe(state.clone(), user1, Json(payload))
            .await
            .unwrap()
            .0;

        // User 2 tries to read it - should fail (forbidden)
        let err = get_recipe(state.clone(), user2, Path(r1.id)).await;
        assert!(matches!(err, Err(AppError::Forbidden)));

        // User 2 tries to update it - should fail
        let payload2 = CreateRecipe {
            name: "User2 Update".to_string(),
            tags: vec![],
            source: None,
            time: None,
            work_minutes: None,
            overall_minutes: None,
            size_number: None,
            size_text: None,
            notes: vec![],
            main_image: None,
            images: vec![],
            sections: vec![],
        };
        let err = update_recipe(state.clone(), user2, Path(r1.id), Json(payload2)).await;
        assert!(matches!(err, Err(AppError::Forbidden)));
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_recipe_copy(pool: PgPool) {
        let state = State(AppState { pool: pool.clone() });
        let user_id_i64 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('copy_u', 'copy@example.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id;
        let user_id = Extension(user_id_i64 as i32);

        let payload = CreateRecipe {
            name: "Original".to_string(),
            tags: vec![],
            source: None,
            time: None,
            work_minutes: Some(10),
            overall_minutes: None,
            size_number: None,
            size_text: None,
            notes: vec!["Test note".to_string()],
            main_image: None,
            images: vec![],
            sections: vec![],
        };
        let orig = create_recipe(state.clone(), user_id, Json(payload))
            .await
            .unwrap()
            .0;

        // Copy recipe
        let copied = copy_recipe(state.clone(), user_id, Path(orig.id))
            .await
            .unwrap()
            .0;

        assert_ne!(orig.id, copied.id); // Different IDs
        assert_eq!(copied.name, "Original (Copy)");
        assert_eq!(copied.work_minutes, Some(10));
        assert_eq!(copied.notes.len(), 1);
        assert_eq!(copied.notes[0], "Test note");
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_search_recipes(pool: PgPool) {
        let state = State(AppState { pool: pool.clone() });
        let user_id_i64 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('search_u', 'search@example.com', 'hash') RETURNING id")
            .fetch_one(&pool).await.unwrap().id;
        let user_id = Extension(user_id_i64 as i32);

        // Recipe 1: 15 mins
        let r1 = CreateRecipe {
            name: "Fast Recipe".to_string(),
            tags: vec![],
            source: None,
            time: None,
            work_minutes: Some(15),
            overall_minutes: None,
            size_number: None,
            size_text: None,
            notes: vec![],
            main_image: None,
            images: vec![],
            sections: vec![],
        };
        let _ = create_recipe(state.clone(), user_id, Json(r1))
            .await
            .unwrap();

        // Recipe 2: 45 mins
        let r2 = CreateRecipe {
            name: "Slow Recipe".to_string(),
            tags: vec![],
            source: None,
            time: None,
            work_minutes: Some(45),
            overall_minutes: None,
            size_number: None,
            size_text: None,
            notes: vec![],
            main_image: None,
            images: vec![],
            sections: vec![],
        };
        let _ = create_recipe(state.clone(), user_id, Json(r2))
            .await
            .unwrap();

        // Search max_work_time = 30
        let filters = crate::models::recipe::RecipeFilters {
            categories: None,
            tags: None,
            ingredients: None,
            max_work_time: Some(30),
            access_rights: None,
            share_states: None,
        };
        let query = RecipeSearchQuery {
            filters: Some(filters),
            sort: None,
        };

        let res = search_recipes(
            state.clone(),
            user_id,
            Query(PaginationQuery {
                page: None,
                limit: None,
            }),
            Some(Json(query)),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(res.data.len(), 1);
        assert_eq!(res.data[0].name, "Fast Recipe");
    }
}

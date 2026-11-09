//! REST API endpoints for recipe management.
//!
//! Provides handlers for listing, creating, reading, updating, deleting,
//! and copying recipes using PostgreSQL via `diesel-async`.
mod helpers;

use crate::models::recipe::{
    CreateRecipe, ImageRef, Order, Recipe, RecipeAccessRights, RecipePreview, RecipeSearchQuery,
    RecipeShareState,
};
use crate::{AppState, error::AppError};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use diesel::prelude::*;
use diesel::result::OptionalExtension;
use diesel::sql_types::Bool;
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::schema::{
    images, recipe_editors, recipe_images, recipe_tags, recipe_viewers, recipes,
    user_category_recipes,
};

/// Dynamically composed WHERE condition over `recipes` for the search query.
type Cond<'a> = Box<dyn BoxableExpression<recipes::table, diesel::pg::Pg, SqlType = Bool> + 'a>;

use chrono::{DateTime, Utc};

/// Row shape for the recipe metadata queries (notes as DB-native `text[]`).
type RecipeRow = (
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    Option<String>,
    Vec<Option<String>>,
    Option<i64>,
    DateTime<Utc>,
    DateTime<Utc>,
);

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

/// Column tuple shared by the recipe metadata queries (matches `RecipeRow`).
macro_rules! recipe_cols {
    () => {
        (
            recipes::id,
            recipes::owner_id,
            recipes::name,
            recipes::source,
            recipes::time_display,
            recipes::work_minutes,
            recipes::overall_minutes,
            recipes::size_number,
            recipes::size_text,
            recipes::notes,
            recipes::main_image_id,
            recipes::created_at,
            recipes::updated_at,
        )
    };
}

async fn get_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(id): Path<i32>,
) -> Result<Json<Recipe>, AppError> {
    let r_id = i64::from(id);
    let u_id = user_id;

    let mut conn = state.pool.get().await?;

    let rec = recipes::table
        .filter(recipes::id.eq(r_id))
        .select(recipe_cols!())
        .first::<RecipeRow>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    let (
        id,
        owner_id,
        name,
        source,
        time_display,
        work_minutes,
        overall_minutes,
        size_number,
        size_text,
        notes,
        main_image_id,
        created_at,
        updated_at,
    ) = rec;

    let editors: Vec<i64> = recipe_editors::table
        .filter(recipe_editors::recipe_id.eq(r_id))
        .select(recipe_editors::user_id)
        .load(&mut conn)
        .await?;

    let viewers: Vec<i64> = recipe_viewers::table
        .filter(recipe_viewers::recipe_id.eq(r_id))
        .select(recipe_viewers::user_id)
        .load(&mut conn)
        .await?;

    if owner_id != u_id && !editors.contains(&u_id) && !viewers.contains(&u_id) {
        return Err(AppError::Forbidden);
    }

    // `myAccessSince` reflects when *this* user got access; the owner's
    // access starts when the recipe was created. Editor beats viewer when a
    // user somehow appears in both lists.
    let my_access_since = if owner_id == u_id {
        Some(created_at.to_rfc3339())
    } else {
        let editor_since: Option<DateTime<Utc>> = recipe_editors::table
            .filter(recipe_editors::recipe_id.eq(r_id))
            .filter(recipe_editors::user_id.eq(u_id))
            .select(recipe_editors::added_at)
            .first(&mut conn)
            .await
            .optional()?;
        let viewer_since: Option<DateTime<Utc>> = recipe_viewers::table
            .filter(recipe_viewers::recipe_id.eq(r_id))
            .filter(recipe_viewers::user_id.eq(u_id))
            .select(recipe_viewers::added_at)
            .first(&mut conn)
            .await
            .optional()?;
        editor_since.or(viewer_since).map(|ts| ts.to_rfc3339())
    };

    let tags: Vec<String> = recipe_tags::table
        .filter(recipe_tags::recipe_id.eq(r_id))
        .order(recipe_tags::position.asc())
        .select(recipe_tags::tag_id)
        .load(&mut conn)
        .await?;

    let main_image: Option<ImageRef> = match main_image_id {
        Some(image_id) => images::table
            .filter(images::id.eq(image_id))
            .select((images::id, images::hash, images::name))
            .first::<(i64, String, Option<String>)>(&mut conn)
            .await
            .optional()?
            .map(|(id, hash, name)| ImageRef { id, hash, name }),
        None => None,
    };

    let gallery_images: Vec<ImageRef> = recipe_images::table
        .inner_join(images::table)
        .filter(recipe_images::recipe_id.eq(r_id))
        .order(recipe_images::position.asc())
        .select((images::id, images::hash, images::name))
        .load(&mut conn)
        .await?
        .into_iter()
        .map(|(id, hash, name)| ImageRef { id, hash, name })
        .collect();

    let sections = helpers::fetch_sections(r_id, &mut conn).await?;

    let recipe = Recipe {
        id: id as i32,
        owner: owner_id as i32,
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
        notes: notes.into_iter().flatten().collect(),
        main_image,
        images: gallery_images,
        sections,
        created_at: Some(created_at.to_rfc3339()),
        updated_at: Some(updated_at.to_rfc3339()),
        my_access_since,
    };
    Ok(Json(recipe))
}

async fn create_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Json(payload): Json<CreateRecipe>,
) -> Result<Json<Recipe>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }

    let u_id = user_id;
    let mut conn = state.pool.get().await?;

    let rec_id = conn
        .transaction::<_, AppError, _>(async move |conn| {
            let rec_id: i64 = diesel::insert_into(recipes::table)
                .values((
                    recipes::owner_id.eq(u_id),
                    recipes::name.eq(payload.name.clone()),
                    recipes::source.eq(payload.source.clone()),
                    recipes::time_display.eq(payload.time.clone()),
                    recipes::work_minutes.eq(payload.work_minutes),
                    recipes::overall_minutes.eq(payload.overall_minutes),
                    recipes::size_number.eq(payload.size_number),
                    recipes::size_text.eq(payload.size_text.clone()),
                    recipes::notes.eq(payload.notes.clone()),
                    recipes::main_image_id.eq(payload.main_image.map(i64::from)),
                ))
                .returning(recipes::id)
                .get_result(conn)
                .await?;

            helpers::insert_recipe_tags(rec_id, payload.tags.clone(), conn).await?;
            helpers::insert_recipe_images(rec_id, payload.images.clone(), conn).await?;

            for (s_pos, sec) in payload.sections.into_iter().enumerate() {
                helpers::insert_section(sec, rec_id, s_pos, conn).await?;
            }

            Ok(rec_id)
        })
        .await?;

    get_recipe(State(state), Extension(user_id), Path(rec_id as i32)).await
}

async fn update_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(id): Path<i32>,
    Json(payload): Json<CreateRecipe>,
) -> Result<Json<Recipe>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }

    let r_id = i64::from(id);
    let u_id = user_id;

    let mut conn = state.pool.get().await?;

    conn.transaction::<_, AppError, _>(async move |conn| {
        helpers::check_can_edit(r_id, u_id, conn).await?;
        helpers::update_recipe_metadata(r_id, &payload, conn).await?;
        helpers::replace_recipe_tags(r_id, payload.tags.clone(), conn).await?;
        helpers::replace_recipe_images(r_id, payload.images.clone(), conn).await?;
        helpers::replace_recipe_sections(r_id, payload.sections.clone(), conn).await?;
        Ok(())
    })
    .await?;

    get_recipe(State(state), Extension(user_id), Path(id)).await
}

async fn delete_recipe(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(id): Path<i32>,
) -> Result<axum::http::StatusCode, AppError> {
    let r_id = i64::from(id);
    let u_id = user_id;

    let mut conn = state.pool.get().await?;

    let res = diesel::delete(recipes::table)
        .filter(recipes::id.eq(r_id))
        .filter(recipes::owner_id.eq(u_id))
        .execute(&mut conn)
        .await?;

    if res == 0 {
        // Either not found or forbidden
        let exists = recipes::table
            .filter(recipes::id.eq(r_id))
            .select(recipes::id)
            .first::<i64>(&mut conn)
            .await
            .optional()?
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
    Extension(user_id): Extension<i64>,
    Path(id): Path<i32>,
) -> Result<Json<Recipe>, AppError> {
    let original = get_recipe(State(state.clone()), Extension(user_id), Path(id))
        .await?
        .0;

    let create_payload = helpers::create_recipe_copy(original);

    create_recipe(State(state), Extension(user_id), Json(create_payload)).await
}

/// Build the visibility guard: owner, editor, or viewer.
fn visibility_guard(u_id: i64) -> Cond<'static> {
    Box::new(
        recipes::owner_id
            .eq(u_id)
            .or(diesel::dsl::exists(
                recipe_editors::table
                    .filter(recipe_editors::recipe_id.eq(recipes::id))
                    .filter(recipe_editors::user_id.eq(u_id)),
            ))
            .or(diesel::dsl::exists(
                recipe_viewers::table
                    .filter(recipe_viewers::recipe_id.eq(recipes::id))
                    .filter(recipe_viewers::user_id.eq(u_id)),
            )),
    )
}

fn nonempty<T>(opt: &Option<Vec<T>>) -> Option<&Vec<T>> {
    opt.as_ref().filter(|v| !v.is_empty())
}

async fn fetch_recipes(
    state: &AppState,
    user_id: i64,
    pagination: PaginationQuery,
    query: RecipeSearchQuery,
) -> Result<PaginatedResponse<RecipePreview>, AppError> {
    let u_id = user_id;
    let limit = pagination.limit.unwrap_or(20).min(100);
    let page = pagination.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;

    let mut conn = state.pool.get().await?;

    let mut q = recipes::table.select(recipe_cols!()).into_boxed();

    // Visibility guard is always applied.
    q = q.filter(visibility_guard(u_id));

    if let Some(f) = &query.filters {
        let has_access = nonempty(&f.access_rights).is_some();
        let has_shares = nonempty(&f.share_states).is_some();

        if has_access || has_shares {
            // OR-combination of the requested access/share conditions,
            // seeded with `false` so an empty request matches nothing.
            let mut or_cond: Cond = Box::new(diesel::dsl::sql::<Bool>("1=0"));

            if let Some(accs) = nonempty(&f.access_rights) {
                for a in accs {
                    let branch: Cond = match a {
                        RecipeAccessRights::Owner => Box::new(recipes::owner_id.eq(u_id)),
                        RecipeAccessRights::Editor => Box::new(diesel::dsl::exists(
                            recipe_editors::table
                                .filter(recipe_editors::recipe_id.eq(recipes::id))
                                .filter(recipe_editors::user_id.eq(u_id)),
                        )),
                        RecipeAccessRights::Viewer => Box::new(diesel::dsl::exists(
                            recipe_viewers::table
                                .filter(recipe_viewers::recipe_id.eq(recipes::id))
                                .filter(recipe_viewers::user_id.eq(u_id)),
                        )),
                    };
                    or_cond = Box::new(or_cond.or(branch));
                }
            }

            if let Some(shs) = nonempty(&f.share_states) {
                for s in shs {
                    let branch: Cond = match s {
                        RecipeShareState::Private => Box::new(
                            recipes::owner_id
                                .eq(u_id)
                                .and(diesel::dsl::not(diesel::dsl::exists(
                                    recipe_editors::table
                                        .filter(recipe_editors::recipe_id.eq(recipes::id)),
                                )))
                                .and(diesel::dsl::not(diesel::dsl::exists(
                                    recipe_viewers::table
                                        .filter(recipe_viewers::recipe_id.eq(recipes::id)),
                                ))),
                        ),
                        RecipeShareState::Shared => Box::new(diesel::dsl::exists(
                            recipe_viewers::table.filter(recipe_viewers::recipe_id.eq(recipes::id)),
                        )),
                        RecipeShareState::Collaborative => Box::new(diesel::dsl::exists(
                            recipe_editors::table.filter(recipe_editors::recipe_id.eq(recipes::id)),
                        )),
                    };
                    or_cond = Box::new(or_cond.or(branch));
                }
            }

            q = q.filter(or_cond);
        }

        // AND-filters: every requested value must be present on the recipe.
        if let Some(cats) = nonempty(&f.categories) {
            let mut cond: Cond = Box::new(diesel::dsl::exists(
                user_category_recipes::table
                    .filter(user_category_recipes::recipe_id.eq(recipes::id))
                    .filter(user_category_recipes::category_id.eq(cats[0])),
            ));
            for c in &cats[1..] {
                cond = Box::new(
                    cond.and(Box::new(diesel::dsl::exists(
                        user_category_recipes::table
                            .filter(user_category_recipes::recipe_id.eq(recipes::id))
                            .filter(user_category_recipes::category_id.eq(*c)),
                    )) as Cond),
                );
            }
            q = q.filter(cond);
        }

        if let Some(tags) = nonempty(&f.tags) {
            let mut cond: Cond = Box::new(diesel::dsl::exists(
                recipe_tags::table
                    .filter(recipe_tags::recipe_id.eq(recipes::id))
                    .filter(recipe_tags::tag_id.eq(tags[0].clone())),
            ));
            for t in &tags[1..] {
                cond = Box::new(
                    cond.and(Box::new(diesel::dsl::exists(
                        recipe_tags::table
                            .filter(recipe_tags::recipe_id.eq(recipes::id))
                            .filter(recipe_tags::tag_id.eq(t.clone())),
                    )) as Cond),
                );
            }
            q = q.filter(cond);
        }

        if let Some(ings) = nonempty(&f.ingredients) {
            let mut cond: Cond = Box::new(diesel::dsl::exists(
                crate::schema::sections::table
                    .inner_join(crate::schema::recipe_ingredients::table)
                    .filter(crate::schema::sections::recipe_id.eq(recipes::id))
                    .filter(crate::schema::recipe_ingredients::ingredient_id.eq(ings[0])),
            ));
            for i in &ings[1..] {
                cond = Box::new(
                    cond.and(Box::new(diesel::dsl::exists(
                        crate::schema::sections::table
                            .inner_join(crate::schema::recipe_ingredients::table)
                            .filter(crate::schema::sections::recipe_id.eq(recipes::id))
                            .filter(crate::schema::recipe_ingredients::ingredient_id.eq(*i)),
                    )) as Cond),
                );
            }
            q = q.filter(cond);
        }

        if let Some(max_time) = f.max_work_time {
            q = q.filter(Box::new(recipes::work_minutes.assume_not_null().le(max_time)) as Cond);
        }
    }

    // Sorting
    if let Some(s) = &query.sort {
        let desc = s.order == Order::Desc;
        match s.field.as_str() {
            "name" => {
                if desc {
                    q = q.then_order_by(recipes::name.desc().nulls_last());
                } else {
                    q = q.then_order_by(recipes::name.asc().nulls_last());
                }
            }
            "worktime" | "work_time" => {
                if desc {
                    q = q.then_order_by(recipes::work_minutes.desc().nulls_last());
                } else {
                    q = q.then_order_by(recipes::work_minutes.asc().nulls_last());
                }
            }
            "totaltime" | "total_time" => {
                if desc {
                    q = q.then_order_by(recipes::overall_minutes.desc().nulls_last());
                } else {
                    q = q.then_order_by(recipes::overall_minutes.asc().nulls_last());
                }
            }
            "rating" => {
                // COALESCE makes the value non-null, so NULLS LAST is moot.
                let dir = if desc { "DESC" } else { "ASC" };
                q = q.then_order_by(diesel::dsl::sql::<diesel::sql_types::Double>(&format!(
                    "(SELECT COALESCE(AVG(rating), 0.0) FROM user_ratings WHERE recipe_id = recipes.id) {dir} NULLS LAST"
                )));
            }
            "access_since" | "accessSince" => {
                // Mirrors the per-row `myAccessSince` computation: owner uses
                // the recipe's creation time, everyone else their join-table
                // `added_at` (editor before viewer).
                let dir = if desc { "DESC" } else { "ASC" };
                q = q.then_order_by(diesel::dsl::sql::<diesel::sql_types::Timestamptz>(
                    &format!(
                        "CASE WHEN recipes.owner_id = {u_id} THEN recipes.created_at ELSE COALESCE(\
                         (SELECT added_at FROM recipe_editors WHERE recipe_id = recipes.id AND user_id = {u_id}), \
                         (SELECT added_at FROM recipe_viewers WHERE recipe_id = recipes.id AND user_id = {u_id}), \
                         recipes.created_at) END {dir}"
                    ),
                ));
            }
            _ => {
                q = q.then_order_by(recipes::id.asc());
            }
        }
    } else {
        q = q.then_order_by(recipes::id.asc());
    }

    let mut records = q
        .limit(limit + 1)
        .offset(offset)
        .load::<RecipeRow>(&mut conn)
        .await?;

    let has_more = records.len() > limit as usize;
    if has_more {
        records.pop();
    }

    let ids: Vec<i64> = records.iter().map(|r| r.0).collect();

    // Batch-load the per-recipe aggregates for the page.
    let editors: Vec<(i64, i64)> = if ids.is_empty() {
        vec![]
    } else {
        recipe_editors::table
            .filter(recipe_editors::recipe_id.eq_any(&ids))
            .select((recipe_editors::recipe_id, recipe_editors::user_id))
            .load(&mut conn)
            .await?
    };

    let viewers: Vec<(i64, i64)> = if ids.is_empty() {
        vec![]
    } else {
        recipe_viewers::table
            .filter(recipe_viewers::recipe_id.eq_any(&ids))
            .select((recipe_viewers::recipe_id, recipe_viewers::user_id))
            .load(&mut conn)
            .await?
    };

    // When the current user was added as editor/viewer, keyed by recipe.
    let my_editor_since: HashMap<i64, DateTime<Utc>> = if ids.is_empty() {
        HashMap::new()
    } else {
        recipe_editors::table
            .filter(recipe_editors::recipe_id.eq_any(&ids))
            .filter(recipe_editors::user_id.eq(u_id))
            .select((recipe_editors::recipe_id, recipe_editors::added_at))
            .load::<(i64, DateTime<Utc>)>(&mut conn)
            .await?
            .into_iter()
            .collect()
    };

    let my_viewer_since: HashMap<i64, DateTime<Utc>> = if ids.is_empty() {
        HashMap::new()
    } else {
        recipe_viewers::table
            .filter(recipe_viewers::recipe_id.eq_any(&ids))
            .filter(recipe_viewers::user_id.eq(u_id))
            .select((recipe_viewers::recipe_id, recipe_viewers::added_at))
            .load::<(i64, DateTime<Utc>)>(&mut conn)
            .await?
            .into_iter()
            .collect()
    };

    let tags: Vec<(i64, String)> = if ids.is_empty() {
        vec![]
    } else {
        recipe_tags::table
            .filter(recipe_tags::recipe_id.eq_any(&ids))
            .order(recipe_tags::position.asc())
            .select((recipe_tags::recipe_id, recipe_tags::tag_id))
            .load(&mut conn)
            .await?
    };

    // Batch-load the main images for the page (join via recipes.main_image_id).
    let main_image_ids: Vec<i64> = records.iter().filter_map(|r| r.10).collect();
    let main_images: Vec<(i64, String, Option<String>)> = if main_image_ids.is_empty() {
        vec![]
    } else {
        images::table
            .filter(images::id.eq_any(&main_image_ids))
            .select((images::id, images::hash, images::name))
            .load(&mut conn)
            .await?
    };
    let image_map: HashMap<i64, ImageRef> = main_images
        .into_iter()
        .map(|(id, hash, name)| (id, ImageRef { id, hash, name }))
        .collect();

    let mut data = Vec::with_capacity(records.len());
    for rec in records {
        let (
            id,
            owner_id,
            name,
            source,
            time_display,
            work_minutes,
            overall_minutes,
            size_number,
            size_text,
            _notes,
            main_image_id,
            created_at,
            updated_at,
        ) = rec;

        let rec_editors: Vec<i32> = editors
            .iter()
            .filter(|(rid, _)| *rid == id)
            .map(|(_, uid)| *uid as i32)
            .collect();
        let rec_viewers: Vec<i32> = viewers
            .iter()
            .filter(|(rid, _)| *rid == id)
            .map(|(_, uid)| *uid as i32)
            .collect();
        let rec_tags: Vec<String> = tags
            .iter()
            .filter(|(rid, _)| *rid == id)
            .map(|(_, tag)| tag.clone())
            .collect();

        let my_access_since = if owner_id == u_id {
            Some(created_at.to_rfc3339())
        } else {
            my_editor_since
                .get(&id)
                .or_else(|| my_viewer_since.get(&id))
                .map(chrono::DateTime::to_rfc3339)
        };

        data.push(RecipePreview {
            id: id as i32,
            owner: owner_id as i32,
            editors: rec_editors,
            viewers: rec_viewers,
            name,
            tags: rec_tags,
            source,
            rating: vec![],
            time: time_display,
            work_minutes,
            overall_minutes,
            size_number,
            size_text,
            main_image: main_image_id.and_then(|id| image_map.get(&id).cloned()),
            created_at: Some(created_at.to_rfc3339()),
            updated_at: Some(updated_at.to_rfc3339()),
            my_access_since,
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
    Extension(user_id): Extension<i64>,
    Query(pagination): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<RecipePreview>>, AppError> {
    let search_query = RecipeSearchQuery::default();
    let res = fetch_recipes(&state, user_id, pagination, search_query).await?;
    Ok(Json(res))
}

async fn search_recipes(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Query(pagination): Query<PaginationQuery>,
    query: Option<Json<RecipeSearchQuery>>,
) -> Result<Json<PaginatedResponse<RecipePreview>>, AppError> {
    let query = query.map(|q| q.0).unwrap_or_default();
    let res = fetch_recipes(&state, user_id, pagination, query).await?;
    Ok(Json(res))
}

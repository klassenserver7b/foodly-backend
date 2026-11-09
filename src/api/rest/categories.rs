use crate::api::rest::recipes::{PaginatedResponse, PaginationQuery};
use crate::models::category::{
    Category, CategoryOrderPair, CategoryOrderResponse, CreateCategory, ReorderCategories,
    UpdateCategoryRecipes,
};
use crate::{AppState, error::AppError};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{get, put},
};
use diesel::prelude::*;
use diesel::result::OptionalExtension;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::schema::user_categories;
use crate::schema::user_category_recipes;

/// Row shape for the raw access-check count query (`QueryableByName` is
/// required by `diesel::sql_query`, which cannot deserialize into tuples).
#[derive(Debug, QueryableByName)]
struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_categories).post(create_category))
        .route("/order", put(reorder_categories))
        .route("/{id}", put(update_category).delete(delete_category))
        .route("/{id}/recipes", put(update_category_recipes))
}

async fn list_categories(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Query(pagination): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<Category>>, AppError> {
    let u_id = user_id;
    let limit = pagination.limit.unwrap_or(20).min(100);
    let page = pagination.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;

    let mut conn = state.pool.get().await?;

    let mut records = user_categories::table
        .filter(user_categories::user_id.eq(u_id))
        .order(user_categories::sort_order.asc())
        .then_order_by(user_categories::id.asc())
        .limit(limit + 1)
        .offset(offset)
        .load::<(
            i64,
            i64,
            String,
            Option<i32>,
            String,
            Option<String>,
            Option<String>,
        )>(&mut conn)
        .await?;

    let has_more = records.len() > limit as usize;
    if has_more {
        records.pop();
    }

    let mut data = Vec::with_capacity(records.len());
    for rec in records {
        // Fetch recipes for this category
        let recipes: Vec<i64> = user_category_recipes::table
            .filter(user_category_recipes::category_id.eq(rec.0))
            .order(user_category_recipes::position.asc())
            .select(user_category_recipes::recipe_id)
            .load(&mut conn)
            .await?;

        data.push(Category {
            id: rec.0,
            user: rec.1,
            name: rec.2.clone(),
            recipes,
            order: rec.3.unwrap_or(0),
            color: rec.4.clone(),
            color_light: rec.5.clone(),
            color_dark: rec.6.clone(),
        });
    }

    let cursor = if has_more {
        Some((page + 1).to_string())
    } else {
        None
    };

    Ok(Json(PaginatedResponse { data, cursor }))
}

async fn create_category(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Json(payload): Json<CreateCategory>,
) -> Result<Json<Category>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }
    if payload.color.is_empty() {
        return Err(AppError::Unprocessable("Color cannot be empty".into()));
    }

    let u_id = user_id;
    let mut conn = state.pool.get().await?;

    // Get current max sort_order for this user
    let max_order: Option<i32> = user_categories::table
        .filter(user_categories::user_id.eq(u_id))
        .select(diesel::dsl::max(user_categories::sort_order))
        .first(&mut conn)
        .await?;

    let new_order = max_order.unwrap_or(-1) + 1;

    let rec = diesel::insert_into(user_categories::table)
        .values((
            user_categories::user_id.eq(u_id),
            user_categories::name.eq(payload.name.clone()),
            user_categories::sort_order.eq(new_order),
            user_categories::color.eq(payload.color.clone()),
            user_categories::color_light.eq(payload.color_light.clone()),
            user_categories::color_dark.eq(payload.color_dark.clone()),
        ))
        .returning(user_categories::id)
        .get_result::<i64>(&mut conn)
        .await?;

    let category = fetch_category(&mut conn, rec).await?;
    Ok(Json(category))
}

async fn update_category(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(id): Path<i32>,
    Json(payload): Json<CreateCategory>,
) -> Result<Json<Category>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }
    if payload.color.is_empty() {
        return Err(AppError::Unprocessable("Color cannot be empty".into()));
    }

    let c_id = i64::from(id);
    let u_id = user_id;
    let mut conn = state.pool.get().await?;

    let res = diesel::update(user_categories::table)
        .filter(user_categories::id.eq(c_id))
        .filter(user_categories::user_id.eq(u_id))
        .set((
            user_categories::name.eq(payload.name.clone()),
            user_categories::color.eq(payload.color.clone()),
            user_categories::color_light.eq(payload.color_light.clone()),
            user_categories::color_dark.eq(payload.color_dark.clone()),
        ))
        .execute(&mut conn)
        .await?;

    if res == 0 {
        return Err(AppError::NotFound("Category not found".into()));
    }

    let category = fetch_category(&mut conn, c_id).await?;
    Ok(Json(category))
}

async fn delete_category(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(id): Path<i32>,
) -> Result<axum::http::StatusCode, AppError> {
    let c_id = i64::from(id);
    let u_id = user_id;
    let mut conn = state.pool.get().await?;

    let res = diesel::delete(user_categories::table)
        .filter(user_categories::id.eq(c_id))
        .filter(user_categories::user_id.eq(u_id))
        .execute(&mut conn)
        .await?;

    if res == 0 {
        return Err(AppError::NotFound("Category not found".into()));
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Upper bound on the recipe IDs accepted in one update request. The
/// handler issues one INSERT per recipe, so an unbounded list would let
/// a single request monopolize a DB connection.
const MAX_RECIPE_IDS: usize = 500;

async fn update_category_recipes(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateCategoryRecipes>,
) -> Result<Json<Category>, AppError> {
    if payload.recipe_ids.len() > MAX_RECIPE_IDS {
        return Err(AppError::Unprocessable(format!(
            "recipeIds accepts at most {MAX_RECIPE_IDS} entries"
        )));
    }

    let c_id = i64::from(id);

    let u_id = user_id;
    let mut conn = state.pool.get().await?;

    // Verify category belongs to user
    let category_exists = user_categories::table
        .filter(user_categories::id.eq(c_id))
        .filter(user_categories::user_id.eq(u_id))
        .select(user_categories::id)
        .first::<i64>(&mut conn)
        .await
        .optional()?
        .is_some();

    if !category_exists {
        return Err(AppError::NotFound("Category not found".into()));
    }

    // Verify user has access to all recipes (owner, editor, or viewer)
    if !payload.recipe_ids.is_empty() {
        let r_ids: Vec<i64> = payload.recipe_ids.to_vec();

        let count = diesel::sql_query(
            r"
            SELECT COUNT(DISTINCT r.id) AS count
            FROM recipes r
            LEFT JOIN recipe_editors e ON r.id = e.recipe_id
            LEFT JOIN recipe_viewers v ON r.id = v.recipe_id
            WHERE r.id = ANY($1)
              AND (r.owner_id = $2 OR e.user_id = $2 OR v.user_id = $2)
            ",
        )
        .bind::<diesel::sql_types::Array<diesel::sql_types::BigInt>, _>(&r_ids)
        .bind::<diesel::sql_types::BigInt, _>(u_id)
        .get_result::<CountRow>(&mut conn)
        .await?;

        if count.count != r_ids.len() as i64 {
            return Err(AppError::Unprocessable(
                "One or more recipes not accessible".into(),
            ));
        }
    }

    // Delete existing mapping
    diesel::delete(user_category_recipes::table)
        .filter(user_category_recipes::category_id.eq(c_id))
        .execute(&mut conn)
        .await?;

    // Insert new mapping
    for (i, r_id) in payload.recipe_ids.iter().enumerate() {
        diesel::insert_into(user_category_recipes::table)
            .values((
                user_category_recipes::category_id.eq(c_id),
                user_category_recipes::recipe_id.eq(*r_id),
                user_category_recipes::position.eq(i as i32),
            ))
            .execute(&mut conn)
            .await?;
    }

    let category = fetch_category(&mut conn, c_id).await?;
    Ok(Json(category))
}

/// Upper bound on the category IDs accepted in one reorder request.
/// The handler issues one UPDATE per ID, so an unbounded list lets a
/// single authenticated request monopolize a pooled DB connection
/// (and its memory) for minutes.
const MAX_REORDER_IDS: usize = 100;

async fn reorder_categories(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Json(payload): Json<ReorderCategories>,
) -> Result<Json<CategoryOrderResponse>, AppError> {
    if payload.category_ids.len() > MAX_REORDER_IDS {
        return Err(AppError::Unprocessable(format!(
            "categoryIds accepts at most {MAX_REORDER_IDS} entries"
        )));
    }

    let u_id = user_id;
    let mut conn = state.pool.get().await?;
    let mut data = Vec::new();

    for (i, cat_id) in payload.category_ids.iter().enumerate() {
        let c_id = *cat_id;
        let order = i as i32;

        let res = diesel::update(user_categories::table)
            .filter(user_categories::id.eq(c_id))
            .filter(user_categories::user_id.eq(u_id))
            .set(user_categories::sort_order.eq(order))
            .execute(&mut conn)
            .await?;

        if res > 0 {
            data.push(CategoryOrderPair { id: *cat_id, order });
        }
    }

    Ok(Json(CategoryOrderResponse { data }))
}

async fn fetch_category(conn: &mut AsyncPgConnection, id: i64) -> Result<Category, AppError> {
    let rec = user_categories::table
        .find(id)
        .first::<(
            i64,
            i64,
            String,
            Option<i32>,
            String,
            Option<String>,
            Option<String>,
        )>(conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("Category not found".into()))?;

    let recipes: Vec<i64> = user_category_recipes::table
        .filter(user_category_recipes::category_id.eq(id))
        .order(user_category_recipes::position.asc())
        .select(user_category_recipes::recipe_id)
        .load(conn)
        .await?;

    Ok(Category {
        id: rec.0,
        user: rec.1,
        name: rec.2.clone(),
        recipes,
        order: rec.3.unwrap_or(0),
        color: rec.4.clone(),
        color_light: rec.5.clone(),
        color_dark: rec.6.clone(),
    })
}

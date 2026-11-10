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
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use crate::schema::user_categories;
use crate::schema::user_category_recipes;

/// Row shape for the raw access-check count query (`QueryableByName` is
/// required by `diesel::sql_query`, which cannot deserialize into tuples).
#[derive(Debug, QueryableByName)]
struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

/// Row shape for the bulk reorder `UPDATE ... RETURNING` query.
#[derive(Debug, QueryableByName)]
struct ReorderedRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    sort_order: i32,
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
    Path(id): Path<i64>,
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

    let res = diesel::update(user_categories::table)
        .filter(user_categories::id.eq(id))
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

    let category = fetch_category(&mut conn, id).await?;
    Ok(Json(category))
}

async fn delete_category(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(id): Path<i64>,
) -> Result<axum::http::StatusCode, AppError> {
    let u_id = user_id;
    let mut conn = state.pool.get().await?;

    let res = diesel::delete(user_categories::table)
        .filter(user_categories::id.eq(id))
        .filter(user_categories::user_id.eq(u_id))
        .execute(&mut conn)
        .await?;

    if res == 0 {
        return Err(AppError::NotFound("Category not found".into()));
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Upper bound on the recipe IDs accepted in one update request, keeping the
/// single bulk INSERT's statement size bounded.
const MAX_RECIPE_IDS: usize = 500;

async fn update_category_recipes(
    State(state): State<AppState>,
    Extension(user_id): Extension<i64>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateCategoryRecipes>,
) -> Result<Json<Category>, AppError> {
    if payload.recipe_ids.len() > MAX_RECIPE_IDS {
        return Err(AppError::Unprocessable(format!(
            "recipeIds accepts at most {MAX_RECIPE_IDS} entries"
        )));
    }

    let u_id = user_id;
    let mut conn = state.pool.get().await?;

    // Verify category belongs to user
    let category_exists = user_categories::table
        .filter(user_categories::id.eq(id))
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

    // Replace the mapping atomically: a failure between DELETE and INSERT
    // must not leave the category emptied.
    let category = conn
        .transaction::<_, AppError, _>(async |conn| {
            diesel::delete(user_category_recipes::table)
                .filter(user_category_recipes::category_id.eq(id))
                .execute(conn)
                .await?;

            let values: Vec<_> = payload
                .recipe_ids
                .iter()
                .enumerate()
                .map(|(i, r_id)| {
                    (
                        user_category_recipes::category_id.eq(id),
                        user_category_recipes::recipe_id.eq(*r_id),
                        user_category_recipes::position.eq(i as i32),
                    )
                })
                .collect();

            diesel::insert_into(user_category_recipes::table)
                .values(&values)
                .execute(conn)
                .await?;

            fetch_category(conn, id).await
        })
        .await?;

    Ok(Json(category))
}

/// Upper bound on the category IDs accepted in one reorder request.
/// The handler updates all rows in a single statement, so the cap keeps
/// the statement (and its bound arrays) reasonably small.
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

    let ids: Vec<i64> = payload.category_ids;
    let ords: Vec<i32> = (0..ids.len() as i32).collect();

    // One round-trip instead of one UPDATE per category id. `unnest` turns the
    // two bound arrays into the (id, ord) value list; every parameter is bound,
    // so nothing user-controlled is interpolated into the SQL.
    let mut rows = diesel::sql_query(
        r"
        UPDATE user_categories AS uc
        SET sort_order = v.ord
        FROM unnest($1::bigint[], $2::int[]) AS v(id, ord)
        WHERE uc.id = v.id AND uc.user_id = $3
        RETURNING uc.id, uc.sort_order
        ",
    )
    .bind::<diesel::sql_types::Array<diesel::sql_types::BigInt>, _>(&ids)
    .bind::<diesel::sql_types::Array<diesel::sql_types::Integer>, _>(&ords)
    .bind::<diesel::sql_types::BigInt, _>(u_id)
    .get_results::<ReorderedRow>(&mut conn)
    .await?;

    // RETURNING has no defined row order; `sort_order` carries the payload
    // position, so sorting by it restores the caller's ordering.
    rows.sort_by_key(|row| row.sort_order);

    let data = rows
        .into_iter()
        .map(|row| CategoryOrderPair {
            id: row.id,
            order: row.sort_order,
        })
        .collect();

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

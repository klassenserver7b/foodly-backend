use crate::models::category::{
    Category, CategoryListResponse, CategoryOrderPair, CategoryOrderResponse, CreateCategory,
    ReorderCategories, UpdateCategoryRecipes,
};
use crate::{AppState, error::AppError};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{get, put},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_categories).post(create_category))
        .route("/order", put(reorder_categories))
        .route("/{id}", put(update_category).delete(delete_category))
        .route("/{id}/recipes", put(update_category_recipes))
}

async fn list_categories(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
) -> Result<Json<CategoryListResponse>, AppError> {
    let u_id = user_id as i64;

    let records = sqlx::query!(
        r#"
        SELECT
            c.id, c.user_id, c.name, c.sort_order, c.color, c.color_light, c.color_dark,
            COALESCE((SELECT array_agg(recipe_id ORDER BY position) FROM user_category_recipes WHERE category_id = c.id), '{}') as "recipes!"
        FROM user_categories c
        WHERE c.user_id = $1
        ORDER BY c.sort_order ASC NULLS LAST, c.id ASC
        "#,
        u_id
    )
    .fetch_all(&state.pool)
    .await?;

    let mut data = Vec::with_capacity(records.len());
    for rec in records {
        data.push(Category {
            id: rec.id as i32,
            user: rec.user_id as i32,
            name: rec.name,
            recipes: rec.recipes.into_iter().map(|v| v as i32).collect(),
            order: rec.sort_order.unwrap_or(0),
            color: rec.color,
            color_light: rec.color_light,
            color_dark: rec.color_dark,
        });
    }

    Ok(Json(CategoryListResponse { data }))
}

async fn create_category(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Json(payload): Json<CreateCategory>,
) -> Result<Json<Category>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }
    if payload.color.is_empty() {
        return Err(AppError::Unprocessable("Color cannot be empty".into()));
    }

    let u_id = user_id as i64;

    // Get current max sort_order
    let max_order: Option<i32> = sqlx::query_scalar!(
        "SELECT MAX(sort_order) FROM user_categories WHERE user_id = $1",
        u_id
    )
    .fetch_one(&state.pool)
    .await?;

    let new_order = max_order.unwrap_or(-1) + 1;

    let rec = sqlx::query!(
        r#"
        INSERT INTO user_categories (user_id, name, sort_order, color, color_light, color_dark)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id, sort_order
        "#,
        u_id,
        payload.name,
        new_order,
        payload.color,
        payload.color_light,
        payload.color_dark
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(Category {
        id: rec.id as i32,
        user: user_id,
        name: payload.name,
        recipes: vec![],
        order: rec.sort_order.unwrap_or(0),
        color: payload.color,
        color_light: payload.color_light,
        color_dark: payload.color_dark,
    }))
}

async fn update_category(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
    Json(payload): Json<CreateCategory>,
) -> Result<Json<Category>, AppError> {
    if payload.name.is_empty() {
        return Err(AppError::Unprocessable("Name cannot be empty".into()));
    }
    if payload.color.is_empty() {
        return Err(AppError::Unprocessable("Color cannot be empty".into()));
    }

    let c_id = id as i64;
    let u_id = user_id as i64;

    let res = sqlx::query!(
        r#"
        UPDATE user_categories
        SET name = $1, color = $2, color_light = $3, color_dark = $4
        WHERE id = $5 AND user_id = $6
        RETURNING id
        "#,
        payload.name,
        payload.color,
        payload.color_light,
        payload.color_dark,
        c_id,
        u_id
    )
    .fetch_optional(&state.pool)
    .await?;

    if res.is_none() {
        return Err(AppError::NotFound("Category not found".into()));
    }

    // Fetch the full category to return
    let rec = sqlx::query!(
        r#"
        SELECT
            c.id, c.user_id, c.name, c.sort_order, c.color, c.color_light, c.color_dark,
            COALESCE((SELECT array_agg(recipe_id ORDER BY position) FROM user_category_recipes WHERE category_id = c.id), '{}') as "recipes!"
        FROM user_categories c
        WHERE c.id = $1
        "#,
        c_id
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(Category {
        id: rec.id as i32,
        user: rec.user_id as i32,
        name: rec.name,
        recipes: rec.recipes.into_iter().map(|v| v as i32).collect(),
        order: rec.sort_order.unwrap_or(0),
        color: rec.color,
        color_light: rec.color_light,
        color_dark: rec.color_dark,
    }))
}

async fn delete_category(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
) -> Result<axum::http::StatusCode, AppError> {
    let c_id = id as i64;
    let u_id = user_id as i64;

    let res = sqlx::query!(
        "DELETE FROM user_categories WHERE id = $1 AND user_id = $2",
        c_id,
        u_id
    )
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("Category not found".into()));
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn update_category_recipes(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateCategoryRecipes>,
) -> Result<Json<Category>, AppError> {
    let c_id = id as i64;
    let u_id = user_id as i64;

    // Verify category belongs to user
    let category_exists = sqlx::query!(
        "SELECT 1 as x FROM user_categories WHERE id = $1 AND user_id = $2",
        c_id,
        u_id
    )
    .fetch_optional(&state.pool)
    .await?
    .is_some();

    if !category_exists {
        return Err(AppError::NotFound("Category not found".into()));
    }

    let mut tx = state.pool.begin().await?;

    // Verify user has access to all recipes (owner, editor, or viewer)
    if !payload.recipe_ids.is_empty() {
        let r_ids: Vec<i64> = payload.recipe_ids.iter().map(|id| *id as i64).collect();

        let valid_count: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT COUNT(DISTINCT r.id)
            FROM recipes r
            LEFT JOIN recipe_editors e ON r.id = e.recipe_id
            LEFT JOIN recipe_viewers v ON r.id = v.recipe_id
            WHERE r.id = ANY($1)
              AND (r.owner_id = $2 OR e.user_id = $2 OR v.user_id = $2)
            "#,
        )
        .bind(&r_ids)
        .bind(u_id)
        .fetch_one(&mut *tx)
        .await?;

        if valid_count.unwrap_or(0) != r_ids.len() as i64 {
            return Err(AppError::Unprocessable(
                "One or more recipes not accessible".into(),
            ));
        }

        // Delete existing mapping
        sqlx::query!(
            "DELETE FROM user_category_recipes WHERE category_id = $1",
            c_id
        )
        .execute(&mut *tx)
        .await?;

        // Insert new mapping
        for (i, r_id) in r_ids.into_iter().enumerate() {
            sqlx::query!(
                "INSERT INTO user_category_recipes (category_id, recipe_id, position) VALUES ($1, $2, $3)",
                c_id,
                r_id,
                i as i32
            )
            .execute(&mut *tx)
            .await?;
        }
    } else {
        // Just clear them
        sqlx::query!(
            "DELETE FROM user_category_recipes WHERE category_id = $1",
            c_id
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    // Return updated category
    let rec = sqlx::query!(
        r#"
        SELECT
            c.id, c.user_id, c.name, c.sort_order, c.color, c.color_light, c.color_dark,
            COALESCE((SELECT array_agg(recipe_id ORDER BY position) FROM user_category_recipes WHERE category_id = c.id), '{}') as "recipes!"
        FROM user_categories c
        WHERE c.id = $1
        "#,
        c_id
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(Category {
        id: rec.id as i32,
        user: rec.user_id as i32,
        name: rec.name,
        recipes: rec.recipes.into_iter().map(|v| v as i32).collect(),
        order: rec.sort_order.unwrap_or(0),
        color: rec.color,
        color_light: rec.color_light,
        color_dark: rec.color_dark,
    }))
}

async fn reorder_categories(
    State(state): State<AppState>,
    Extension(user_id): Extension<i32>,
    Json(payload): Json<ReorderCategories>,
) -> Result<Json<CategoryOrderResponse>, AppError> {
    let u_id = user_id as i64;
    let mut tx = state.pool.begin().await?;

    let mut data = Vec::with_capacity(payload.category_ids.len());

    for (i, cat_id) in payload.category_ids.into_iter().enumerate() {
        let c_id = cat_id as i64;
        let order = i as i32;

        let res = sqlx::query!(
            "UPDATE user_categories SET sort_order = $1 WHERE id = $2 AND user_id = $3",
            order,
            c_id,
            u_id
        )
        .execute(&mut *tx)
        .await?;

        if res.rows_affected() > 0 {
            data.push(CategoryOrderPair { id: cat_id, order });
        }
    }

    tx.commit().await?;

    Ok(Json(CategoryOrderResponse { data }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};
    use axum::{Extension, Json};
    use sqlx::PgPool;

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_category_lifecycle(pool: PgPool) {
        let state = State(AppState { pool: pool.clone() });

        let user_id_i64 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('cat_user', 'cat@example.com', 'hash') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap()
            .id;
        let user_id = Extension(user_id_i64 as i32);

        // 1. Create a category
        let create_payload = CreateCategory {
            name: "Test Cat".to_string(),
            color: "#ff0000".to_string(),
            color_light: None,
            color_dark: None,
        };

        let cat1 = create_category(state.clone(), user_id.clone(), Json(create_payload))
            .await
            .unwrap()
            .0;

        assert_eq!(cat1.name, "Test Cat");
        assert_eq!(cat1.order, 0);
        let c1_id = cat1.id;

        // Create another category to test order
        let create_payload_2 = CreateCategory {
            name: "Test Cat 2".to_string(),
            color: "#00ff00".to_string(),
            color_light: None,
            color_dark: None,
        };
        let cat2 = create_category(state.clone(), user_id.clone(), Json(create_payload_2))
            .await
            .unwrap()
            .0;
        assert_eq!(cat2.order, 1);
        let c2_id = cat2.id;

        // 2. List categories
        let list = list_categories(state.clone(), user_id.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(list.data.len(), 2);
        assert_eq!(list.data[0].id, c1_id);
        assert_eq!(list.data[1].id, c2_id);

        // 3. Update a category
        let update_payload = CreateCategory {
            name: "Test Cat Updated".to_string(),
            color: "#0000ff".to_string(),
            color_light: Some("#ffffff".to_string()),
            color_dark: None,
        };
        let updated = update_category(
            state.clone(),
            user_id.clone(),
            Path(c1_id),
            Json(update_payload),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(updated.name, "Test Cat Updated");
        assert_eq!(updated.color_light, Some("#ffffff".to_string()));

        // 4. Reorder
        let reorder_payload = ReorderCategories {
            category_ids: vec![c2_id, c1_id],
        };
        let order_res = reorder_categories(state.clone(), user_id.clone(), Json(reorder_payload))
            .await
            .unwrap()
            .0;
        assert_eq!(order_res.data[0].id, c2_id);
        assert_eq!(order_res.data[0].order, 0);
        assert_eq!(order_res.data[1].id, c1_id);
        assert_eq!(order_res.data[1].order, 1);

        // 5. Delete
        let _ = delete_category(state.clone(), user_id.clone(), Path(c1_id))
            .await
            .unwrap();
        let list2 = list_categories(state.clone(), user_id.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(list2.data.len(), 1);
    }

    #[sqlx::test(migrations = "src/db/migrations")]
    async fn test_category_recipes(pool: PgPool) {
        let state = State(AppState { pool: pool.clone() });

        let user_id_i64 = sqlx::query!("INSERT INTO users (name, email, password_hash) VALUES ('catr_user', 'catr@example.com', 'hash') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap()
            .id;
        let user_id = Extension(user_id_i64 as i32);

        // Create recipe
        let r_id: i64 = sqlx::query!(
            "INSERT INTO recipes (owner_id, name, notes) VALUES ($1, 'Rec', '{}') RETURNING id",
            user_id_i64
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .id;

        // Create category
        let c_id: i64 = sqlx::query!("INSERT INTO user_categories (user_id, name, color) VALUES ($1, 'Cat', '#000') RETURNING id", user_id_i64).fetch_one(&pool).await.unwrap().id;

        // Update recipes
        let payload = UpdateCategoryRecipes {
            recipe_ids: vec![r_id as i32],
        };
        let cat = update_category_recipes(
            state.clone(),
            user_id.clone(),
            Path(c_id as i32),
            Json(payload),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(cat.recipes.len(), 1);
        assert_eq!(cat.recipes[0], r_id as i32);
    }
}

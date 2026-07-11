use crate::error::AppError;
use crate::models::recipe::{
    CreateRecipe, CreateSection, IngredientRef, Recipe, RecipeIngredient, Section,
};

pub(super) async fn insert_section(
    sec: CreateSection,
    r_id: i64,
    s_pos: usize,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
    let sec_id = sqlx::query!(
        "INSERT INTO sections (recipe_id, name, position) VALUES ($1, $2, $3) RETURNING id",
        r_id,
        sec.name,
        s_pos as i32
    )
    .fetch_one(&mut **tx)
    .await?
    .id;

    for (st_pos, step) in sec.steps.into_iter().enumerate() {
        sqlx::query!(
            "INSERT INTO steps (section_id, text, position) VALUES ($1, $2, $3)",
            sec_id,
            step,
            st_pos as i32
        )
        .execute(&mut **tx)
        .await?;
    }
    for (i_pos, ing) in sec.ingredients.into_iter().enumerate() {
        if !ing.is_valid() {
            return Err(AppError::Unprocessable(format!(
                "Invalid ingredient: {:?}",
                ing
            )));
        }

        sqlx::query!("INSERT INTO recipe_ingredients (section_id, ingredient_id, text, amount, amount_prefix, unit, position) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            sec_id, ing.ingredient.map(|i| i as i64), ing.text, ing.amount, ing.amount_prefix, ing.unit, i_pos as i32).execute(&mut **tx).await?;
    }
    Ok(())
}

pub(super) async fn fetch_sections(
    r_id: i64,
    pool: &sqlx::PgPool,
) -> Result<Vec<Section>, AppError> {
    let sec_rows = sqlx::query!(
        "SELECT id, name FROM sections WHERE recipe_id = $1 ORDER BY position ASC",
        r_id
    )
    .fetch_all(pool)
    .await?;

    let mut sections = Vec::with_capacity(sec_rows.len());
    for sec in sec_rows {
        let step_rows = sqlx::query!(
            "SELECT text FROM steps WHERE section_id = $1 ORDER BY position ASC",
            sec.id
        )
        .fetch_all(pool)
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
        ).fetch_all(pool).await?;

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
    Ok(sections)
}

pub(super) async fn insert_recipe_tags(
    r_id: i64,
    tags: Vec<String>,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
    for (pos, tag) in tags.into_iter().enumerate() {
        sqlx::query!(
            "INSERT INTO recipe_tags (recipe_id, tag_id, position) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            r_id,
            tag,
            pos as i32
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub(super) async fn insert_recipe_images(
    r_id: i64,
    images: Vec<i32>,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
    for (pos, img) in images.into_iter().enumerate() {
        sqlx::query!(
            "INSERT INTO recipe_images (recipe_id, image_id, position) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            r_id,
            img as i64,
            pos as i32
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub(super) async fn replace_recipe_tags(
    r_id: i64,
    tags: Vec<String>,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
    sqlx::query!("DELETE FROM recipe_tags WHERE recipe_id = $1", r_id)
        .execute(&mut **tx)
        .await?;
    insert_recipe_tags(r_id, tags, tx).await
}

pub(super) async fn replace_recipe_images(
    r_id: i64,
    images: Vec<i32>,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
    sqlx::query!("DELETE FROM recipe_images WHERE recipe_id = $1", r_id)
        .execute(&mut **tx)
        .await?;
    insert_recipe_images(r_id, images, tx).await
}

pub(super) async fn replace_recipe_sections(
    r_id: i64,
    sections: Vec<CreateSection>,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
    sqlx::query!("DELETE FROM sections WHERE recipe_id = $1", r_id)
        .execute(&mut **tx)
        .await?;
    for (s_pos, sec) in sections.into_iter().enumerate() {
        insert_section(sec, r_id, s_pos, tx).await?;
    }
    Ok(())
}

pub(super) async fn check_can_edit(
    r_id: i64,
    u_id: i64,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
    let owner_check = sqlx::query!(
        "SELECT owner_id, EXISTS(SELECT 1 FROM recipe_editors WHERE recipe_id = $1 AND user_id = $2) as is_editor FROM recipes WHERE id = $1",
        r_id, u_id
    ).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    if owner_check.owner_id != u_id && !owner_check.is_editor.unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub(super) async fn update_recipe_metadata(
    r_id: i64,
    payload: &CreateRecipe,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
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
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) fn create_recipe_copy(original: Recipe) -> CreateRecipe {
    CreateRecipe {
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
            .map(|s| CreateSection {
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
    }
}

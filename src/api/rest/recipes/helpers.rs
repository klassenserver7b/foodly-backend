use crate::error::AppError;
use crate::models::recipe::{
    CreateRecipe, CreateSection, IngredientRef, Recipe, RecipeIngredient, Section,
};
use diesel::prelude::*;
use diesel::result::OptionalExtension;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::schema::{
    ingredients, recipe_editors, recipe_images, recipe_ingredients, recipe_tags, recipes, sections,
    steps,
};

/// Upper bound on ingredients summed across all sections of a recipe.
///
/// Ingredient lists used to be inserted one row per statement; an unbounded
/// list meant tens of thousands of sequential round-trips inside a single
/// transaction. The batch insert below removes that cost, and this cap keeps
/// the statement size itself bounded.
pub(super) const MAX_RECIPE_ITEMS: usize = 1000;

/// Rejects payloads whose combined ingredient count exceeds [`MAX_RECIPE_ITEMS`].
pub(super) fn validate_recipe_limits(payload: &CreateRecipe) -> Result<(), AppError> {
    let total: usize = payload
        .sections
        .iter()
        .map(|section| section.ingredients.len())
        .sum();
    if total > MAX_RECIPE_ITEMS {
        return Err(AppError::Unprocessable(format!(
            "sections accept at most {MAX_RECIPE_ITEMS} ingredients in total"
        )));
    }
    Ok(())
}

pub(super) async fn insert_section(
    sec: CreateSection,
    r_id: i64,
    s_pos: usize,
    conn: &mut AsyncPgConnection,
) -> Result<(), AppError> {
    let sec_id = diesel::insert_into(sections::table)
        .values((
            sections::recipe_id.eq(r_id),
            sections::name.eq(sec.name),
            sections::position.eq(s_pos as i32),
        ))
        .returning(sections::id)
        .get_result::<i64>(conn)
        .await?;

    if !sec.steps.is_empty() {
        let values: Vec<_> = sec
            .steps
            .into_iter()
            .enumerate()
            .map(|(pos, text)| {
                (
                    steps::section_id.eq(sec_id),
                    steps::text.eq(text),
                    steps::position.eq(pos as i32),
                )
            })
            .collect();
        diesel::insert_into(steps::table)
            .values(&values)
            .execute(conn)
            .await?;
    }

    for ing in &sec.ingredients {
        if !ing.is_valid() {
            return Err(AppError::Unprocessable(format!(
                "Invalid ingredient: {ing:?}"
            )));
        }
    }

    if !sec.ingredients.is_empty() {
        // One multi-row statement: a single round-trip regardless of item count.
        let values: Vec<_> = sec
            .ingredients
            .iter()
            .enumerate()
            .map(|(i_pos, ing)| {
                (
                    recipe_ingredients::section_id.eq(sec_id),
                    recipe_ingredients::ingredient_id.eq(ing.ingredient),
                    recipe_ingredients::text.eq(ing.text.clone()),
                    recipe_ingredients::amount.eq(ing.amount.clone()),
                    recipe_ingredients::amount_prefix.eq(ing.amount_prefix.clone()),
                    recipe_ingredients::unit.eq(ing.unit.clone()),
                    recipe_ingredients::position.eq(i_pos as i32),
                )
            })
            .collect();
        diesel::insert_into(recipe_ingredients::table)
            .values(&values)
            .execute(conn)
            .await?;
    }
    Ok(())
}

pub(super) async fn fetch_sections(
    r_id: i64,
    conn: &mut AsyncPgConnection,
) -> Result<Vec<Section>, AppError> {
    let sec_rows = sections::table
        .filter(sections::recipe_id.eq(r_id))
        .order(sections::position.asc())
        .select((sections::id, sections::name))
        .load::<(i64, Option<String>)>(conn)
        .await?;

    let mut result = Vec::with_capacity(sec_rows.len());
    for (sec_id, sec_name) in sec_rows {
        let step_rows: Vec<String> = steps::table
            .filter(steps::section_id.eq(sec_id))
            .order(steps::position.asc())
            .select(steps::text)
            .load(conn)
            .await?;

        let ing_rows = recipe_ingredients::table
            .left_join(ingredients::table)
            .filter(recipe_ingredients::section_id.eq(sec_id))
            .order(recipe_ingredients::position.asc())
            .select((
                recipe_ingredients::id,
                recipe_ingredients::ingredient_id,
                ingredients::name.nullable(),
                recipe_ingredients::text,
                recipe_ingredients::amount,
                recipe_ingredients::amount_prefix,
                recipe_ingredients::unit,
            ))
            .load::<(
                i64,
                Option<i64>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            )>(conn)
            .await?;

        let ings: Vec<RecipeIngredient> = ing_rows
            .into_iter()
            .map(
                |(id, ingredient_id, ingredient_name, text, amount, amount_prefix, unit)| {
                    RecipeIngredient {
                        id,
                        ingredient: ingredient_id.map(|iid| IngredientRef {
                            id: iid,
                            name: ingredient_name.unwrap_or_default(),
                        }),
                        text,
                        amount,
                        amount_prefix,
                        unit,
                    }
                },
            )
            .collect();

        result.push(Section {
            id: sec_id,
            name: sec_name,
            steps: step_rows,
            ingredients: ings,
        });
    }
    Ok(result)
}

pub(super) async fn insert_recipe_tags(
    r_id: i64,
    tags: Vec<String>,
    conn: &mut AsyncPgConnection,
) -> Result<(), AppError> {
    if tags.is_empty() {
        return Ok(());
    }
    let values: Vec<_> = tags
        .into_iter()
        .enumerate()
        .map(|(pos, tag)| {
            (
                recipe_tags::recipe_id.eq(r_id),
                recipe_tags::tag_id.eq(tag),
                recipe_tags::position.eq(pos as i32),
            )
        })
        .collect();
    diesel::insert_into(recipe_tags::table)
        .values(&values)
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    Ok(())
}

pub(super) async fn insert_recipe_images(
    r_id: i64,
    images: Vec<i64>,
    conn: &mut AsyncPgConnection,
) -> Result<(), AppError> {
    if images.is_empty() {
        return Ok(());
    }
    let values: Vec<_> = images
        .into_iter()
        .enumerate()
        .map(|(pos, img)| {
            (
                recipe_images::recipe_id.eq(r_id),
                recipe_images::image_id.eq(img),
                recipe_images::position.eq(pos as i32),
            )
        })
        .collect();
    diesel::insert_into(recipe_images::table)
        .values(&values)
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    Ok(())
}

pub(super) async fn replace_recipe_tags(
    r_id: i64,
    tags: Vec<String>,
    conn: &mut AsyncPgConnection,
) -> Result<(), AppError> {
    diesel::delete(recipe_tags::table)
        .filter(recipe_tags::recipe_id.eq(r_id))
        .execute(conn)
        .await?;
    insert_recipe_tags(r_id, tags, conn).await
}

pub(super) async fn replace_recipe_images(
    r_id: i64,
    images: Vec<i64>,
    conn: &mut AsyncPgConnection,
) -> Result<(), AppError> {
    diesel::delete(recipe_images::table)
        .filter(recipe_images::recipe_id.eq(r_id))
        .execute(conn)
        .await?;
    insert_recipe_images(r_id, images, conn).await
}

pub(super) async fn replace_recipe_sections(
    r_id: i64,
    secs: Vec<CreateSection>,
    conn: &mut AsyncPgConnection,
) -> Result<(), AppError> {
    diesel::delete(sections::table)
        .filter(sections::recipe_id.eq(r_id))
        .execute(conn)
        .await?;
    for (s_pos, sec) in secs.into_iter().enumerate() {
        insert_section(sec, r_id, s_pos, conn).await?;
    }
    Ok(())
}

pub(super) async fn check_can_edit(
    r_id: i64,
    u_id: i64,
    conn: &mut AsyncPgConnection,
) -> Result<(), AppError> {
    let owner_id: i64 = recipes::table
        .filter(recipes::id.eq(r_id))
        .select(recipes::owner_id)
        .first(conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("Recipe not found".into()))?;

    let is_editor: bool = diesel::select(diesel::dsl::exists(
        recipe_editors::table
            .filter(recipe_editors::recipe_id.eq(r_id))
            .filter(recipe_editors::user_id.eq(u_id)),
    ))
    .first(conn)
    .await?;

    if owner_id != u_id && !is_editor {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub(super) async fn update_recipe_metadata(
    r_id: i64,
    payload: &CreateRecipe,
    conn: &mut AsyncPgConnection,
) -> Result<(), AppError> {
    diesel::update(recipes::table.find(r_id))
        .set((
            recipes::name.eq(payload.name.clone()),
            recipes::source.eq(payload.source.clone()),
            recipes::time_display.eq(payload.time.clone()),
            recipes::work_minutes.eq(payload.work_minutes),
            recipes::overall_minutes.eq(payload.overall_minutes),
            recipes::size_number.eq(payload.size_number),
            recipes::size_text.eq(payload.size_text.clone()),
            recipes::notes.eq(payload.notes.clone()),
            recipes::main_image_id.eq(payload.main_image),
            recipes::updated_at.eq(diesel::dsl::now),
        ))
        .execute(conn)
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
        main_image: original.main_image.map(|img| img.id),
        images: original.images.into_iter().map(|img| img.id).collect(),
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

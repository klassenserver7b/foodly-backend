use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Category {
    pub id: i32,
    pub user: i32,
    pub name: String,
    pub recipes: Vec<i32>,
    pub order: i32,
    pub color: String,
    #[serde(rename = "colorLight")]
    pub color_light: Option<String>,
    #[serde(rename = "colorDark")]
    pub color_dark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCategory {
    pub name: String,
    pub color: String,
    #[serde(rename = "colorLight")]
    pub color_light: Option<String>,
    #[serde(rename = "colorDark")]
    pub color_dark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCategoryRecipes {
    #[serde(rename = "recipeIds")]
    pub recipe_ids: Vec<i32>,
}

#[derive(Debug, Deserialize)]
pub struct ReorderCategories {
    #[serde(rename = "categoryIds")]
    pub category_ids: Vec<i32>,
}

#[derive(Debug, Serialize)]
pub struct CategoryOrderPair {
    pub id: i32,
    pub order: i32,
}

#[derive(Debug, Serialize)]
pub struct CategoryOrderResponse {
    pub data: Vec<CategoryOrderPair>,
}

#[derive(Debug, Serialize)]
pub struct CategoryListResponse {
    pub data: Vec<Category>,
}

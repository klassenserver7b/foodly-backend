use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRating {
    pub user: i32,
    pub rating: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngredientRef {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeIngredient {
    #[serde(skip_deserializing, default)]
    pub id: i32,
    pub ingredient: Option<IngredientRef>,
    pub text: Option<String>,
    pub amount: Option<String>,
    pub amount_prefix: Option<String>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    #[serde(skip_deserializing, default)]
    pub id: i32,
    pub name: Option<String>,
    pub ingredients: Vec<RecipeIngredient>,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: i32,
    pub owner: i32,
    #[serde(default)]
    pub editors: Vec<i32>,
    #[serde(default)]
    pub viewers: Vec<i32>,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub rating: Vec<UserRating>,
    pub time: Option<String>,
    pub work_minutes: Option<i32>,
    pub overall_minutes: Option<i32>,
    pub size_number: Option<i32>,
    pub size_text: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub main_image: Option<i32>,
    #[serde(default)]
    pub images: Vec<i32>,
    #[serde(default)]
    pub sections: Vec<Section>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

// For List endpoint
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipePreview {
    pub id: i32,
    pub owner: i32,
    pub editors: Vec<i32>,
    pub viewers: Vec<i32>,
    pub name: String,
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub rating: Vec<UserRating>,
    pub time: Option<String>,
    pub work_minutes: Option<i32>,
    pub overall_minutes: Option<i32>,
    pub size_number: Option<i32>,
    pub size_text: Option<String>,
    pub main_image: Option<i32>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl From<Recipe> for RecipePreview {
    fn from(recipe: Recipe) -> Self {
        Self {
            id: recipe.id,
            owner: recipe.owner,
            editors: recipe.editors,
            viewers: recipe.viewers,
            name: recipe.name,
            tags: recipe.tags,
            source: recipe.source,
            rating: recipe.rating,
            time: recipe.time,
            work_minutes: recipe.work_minutes,
            overall_minutes: recipe.overall_minutes,
            size_number: recipe.size_number,
            size_text: recipe.size_text,
            main_image: recipe.main_image,
            created_at: recipe.created_at,
            updated_at: recipe.updated_at,
        }
    }
}

// Input Models
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecipeIngredient {
    pub ingredient: Option<i32>,
    pub text: Option<String>,
    pub amount: Option<String>,
    pub amount_prefix: Option<String>,
    pub unit: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSection {
    pub name: Option<String>,
    pub ingredients: Vec<CreateRecipeIngredient>,
    pub steps: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecipe {
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub time: Option<String>,
    pub work_minutes: Option<i32>,
    pub overall_minutes: Option<i32>,
    pub size_number: Option<i32>,
    pub size_text: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub main_image: Option<i32>,
    #[serde(default)]
    pub images: Vec<i32>,
    #[serde(default)]
    pub sections: Vec<CreateSection>,
}

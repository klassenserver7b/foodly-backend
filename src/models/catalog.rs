//! Catalog data models (Tags, Ingredients).

use serde::Serialize;

#[derive(Serialize)]
pub struct Tag {
    pub id: String,
    pub svg: Option<String>,
}

#[derive(Serialize)]
pub struct Ingredient {
    pub id: i32,
    pub name: String,
}

#[derive(Serialize)]
pub struct CatalogResponse<T> {
    pub data: Vec<T>,
}

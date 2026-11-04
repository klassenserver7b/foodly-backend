use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSearchQuery {
    pub filters: Option<RecipeFilters>,
    pub sort: Option<RecipeSort>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeFilters {
    pub categories: Option<Vec<i64>>,
    pub tags: Option<Vec<String>>,
    pub ingredients: Option<Vec<i64>>,
    pub max_work_time: Option<i32>,
    pub access_rights: Option<Vec<String>>,
    pub share_states: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSort {
    pub field: String,
    pub order: String,
}

fn main() {
    let json = r#"{
        "page": 1,
        "limit": 10,
        "filters": {
          "maxWorkTime": 30,
          "tags": ["Hauptgericht", "Vegan"],
         "shareStates": ["private", "shared"]
        },
        "sort": {
          "field": "rating",
         "order": "desc"
        }
      }"#;
      
    let query: RecipeSearchQuery = serde_json::from_str(json).unwrap();
    println!("{:?}", query);
}

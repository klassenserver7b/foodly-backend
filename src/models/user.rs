use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub profile_picture: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserUpdatePayload {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserPutPayload {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct UserSearchQuery {
    pub q: String,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSearchItem {
    pub id: i64,
    pub name: String,
    pub profile_picture: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UserSearchResponse {
    pub data: Vec<UserSearchItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePictureResponse {
    pub profile_picture: String,
}

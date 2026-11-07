//! Authentication payload and response models.

use serde::{Deserialize, Serialize};

use crate::models::user::User;

/// POST /api/v1/auth/register request body.
#[derive(Debug, Deserialize)]
pub struct RegisterPayload {
    pub name: String,
    pub email: String,
    pub password: String,
}

/// POST /api/v1/auth/login request body.
#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub email: String,
    pub password: String,
}

/// POST /api/v1/auth/refresh request body.
#[derive(Debug, Deserialize)]
pub struct RefreshPayload {
    #[serde(rename = "refreshToken")]
    pub refresh_token: String,
}

/// Response for register/login: user + access token + refresh token.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserWithTokens {
    pub user: User,
    pub access_token: String,
    pub refresh_token: String,
}

/// Response for refresh: new access + refresh token pair.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPairResponse {
    pub access_token: String,
    pub refresh_token: String,
}

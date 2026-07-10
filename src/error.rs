//! Global error handling structures and `IntoResponse` implementations.
//!
//! Provides the unified `AppError` type used throughout the API handlers
//! to map internal domain/infrastructure errors into standard HTTP responses.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Unified application error type returned by Axum handlers.
///
/// Wraps both explicit user-facing errors (like `NotFound`) and internal
/// server errors (via `anyhow::Error`). Implements `IntoResponse` to
/// serialize cleanly into a JSON error body.
#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Forbidden,
    Unprocessable(String),
    Conflict(String),
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            AppError::Forbidden => (
                StatusCode::FORBIDDEN,
                "FORBIDDEN",
                "Access denied".to_string(),
            ),
            AppError::Unprocessable(msg) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE", msg)
            }
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg),
            AppError::Internal(err) => {
                eprintln!("Internal error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_SERVER_ERROR",
                    "An internal server error occurred".to_string(),
                )
            }
        };

        let body = Json(json!({
            "error": {
                "code": code,
                "message": message
            }
        }));

        (status, body).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        if matches!(err, sqlx::Error::RowNotFound) {
            return Self::NotFound("Resource not found".into());
        }

        if let sqlx::Error::Database(db_err) = &err
            && let Some(code) = db_err.code()
        {
            // Postgres error codes: https://www.postgresql.org/docs/current/errcodes-appendix.html
            match code.as_ref() {
                "22001" => return Self::Unprocessable("Value too long for column".into()),
                "23502" => return Self::Unprocessable("Missing required field".into()),
                "23503" => return Self::Unprocessable("Invalid reference".into()),
                "23505" => return Self::Conflict("Duplicate value".into()),
                "23514" => return Self::Unprocessable("Check constraint violation".into()),
                _ => {}
            }
        }
        Self::Internal(err.into())
    }
}

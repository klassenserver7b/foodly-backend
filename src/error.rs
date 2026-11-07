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
    Unauthorized(String),
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
            AppError::Unauthorized(ref msg) => {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg.clone())
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

impl From<diesel::result::Error> for AppError {
    fn from(err: diesel::result::Error) -> Self {
        use diesel::result::{DatabaseErrorKind, Error as DieselError};

        match err {
            DieselError::NotFound => Self::NotFound("Resource not found".into()),
            DieselError::DatabaseError(kind, ref info) => {
                let message = info.message().to_string();
                match kind {
                    DatabaseErrorKind::UniqueViolation => Self::Conflict("Duplicate value".into()),
                    DatabaseErrorKind::NotNullViolation => {
                        Self::Unprocessable("Missing required field".into())
                    }
                    DatabaseErrorKind::ForeignKeyViolation => {
                        Self::Unprocessable("Invalid reference".into())
                    }
                    DatabaseErrorKind::CheckViolation => {
                        Self::Unprocessable("Check constraint violation".into())
                    }
                    _ => Self::Internal(anyhow::anyhow!("Database error: {}", message)),
                }
            }
            _ => Self::Internal(anyhow::anyhow!("Query error: {}", err)),
        }
    }
}

impl From<diesel_async::pooled_connection::deadpool::PoolError> for AppError {
    fn from(err: diesel_async::pooled_connection::deadpool::PoolError) -> Self {
        use diesel_async::pooled_connection::PoolError as DieselPoolError;
        use diesel_async::pooled_connection::deadpool::PoolError as DeadpoolError;

        match err {
            DeadpoolError::Backend(DieselPoolError::ConnectionError(e)) => {
                Self::Internal(anyhow::anyhow!("DB connection error: {}", e))
            }
            DeadpoolError::Backend(DieselPoolError::QueryError(e)) => {
                Self::Internal(anyhow::anyhow!("DB pool query error: {}", e))
            }
            DeadpoolError::Timeout(_) => {
                Self::Internal(anyhow::anyhow!("Timed out waiting for a DB connection"))
            }
            DeadpoolError::Closed => {
                Self::Internal(anyhow::anyhow!("DB connection pool is closed"))
            }
            DeadpoolError::NoRuntimeSpecified => {
                Self::Internal(anyhow::anyhow!("DB pool: no async runtime specified"))
            }
            e => Self::Internal(anyhow::anyhow!("DB pool error: {}", e)),
        }
    }
}

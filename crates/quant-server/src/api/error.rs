//! API errors: a stable machine-readable `error` code (the UI translates it) plus an English
//! `message` for logs and API users. Internal errors are logged in full and returned generically.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use quant_core::strategy::FieldError;
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Validation(Vec<FieldError>),
    Unauthorized,
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    /// The selected portfolio does not exist, is archived, or is not visible to the user.
    PortfolioNotFound,
    /// The user cannot see any active portfolio.
    NoPortfolio,
    /// Seconds until the caller may retry.
    TooManyRequests(u64),
    Unavailable(String),
    Internal(anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn bad(message: impl Into<String>) -> Self {
        ApiError::BadRequest(message.into())
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        ApiError::NotFound(what.into())
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        ApiError::Conflict(message.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        ApiError::Internal(error)
    }
}

impl From<tokio_postgres::Error> for ApiError {
    fn from(error: tokio_postgres::Error) -> Self {
        ApiError::Internal(error.into())
    }
}

impl From<deadpool_postgres::PoolError> for ApiError {
    fn from(error: deadpool_postgres::PoolError) -> Self {
        ApiError::Unavailable(format!("database unavailable: {error}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                json!({"error": "bad_request", "message": message}),
            ),
            ApiError::Validation(fields) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({"error": "validation", "message": "invalid parameters", "fields": fields}),
            ),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                json!({"error": "unauthorized", "message": "sign in required"}),
            ),
            ApiError::Forbidden(message) => (
                StatusCode::FORBIDDEN,
                json!({"error": "forbidden", "message": message}),
            ),
            ApiError::NotFound(what) => (
                StatusCode::NOT_FOUND,
                json!({"error": "not_found", "message": format!("{what} not found")}),
            ),
            ApiError::Conflict(message) => (
                StatusCode::CONFLICT,
                json!({"error": "conflict", "message": message}),
            ),
            ApiError::PortfolioNotFound => (
                StatusCode::NOT_FOUND,
                json!({"error": "portfolio_not_found", "message": "that portfolio does not exist or is not available"}),
            ),
            ApiError::NoPortfolio => (
                StatusCode::NOT_FOUND,
                json!({"error": "no_portfolio", "message": "no portfolio is available; create one first"}),
            ),
            ApiError::TooManyRequests(retry) => (
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error": "rate_limited", "message": "too many attempts", "retry_after_secs": retry}),
            ),
            ApiError::Unavailable(message) => {
                tracing::warn!(%message, "service unavailable");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"error": "unavailable", "message": message}),
                )
            }
            ApiError::Internal(error) => {
                tracing::error!(error = %format!("{error:#}"), "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error": "internal", "message": "internal error; see server logs"}),
                )
            }
        };
        (status, Json(body)).into_response()
    }
}

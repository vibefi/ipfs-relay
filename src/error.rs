use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use uuid::Uuid;

use crate::middleware::request_id::current_request_id;

/// Application-level error type.
/// All variants produce structured JSON error responses.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("invalid package: {0}")]
    InvalidPackage(String),

    #[error("payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("rate limit exceeded")]
    RateLimitExceeded,

    #[error("not found")]
    NotFound,

    #[error("ipfs error: {0}")]
    Ipfs(#[from] IpfsError),

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum IpfsError {
    #[error("kubo request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("kubo response error: {message}")]
    Api { message: String },
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
    #[serde(rename = "requestId")]
    request_id: String,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl AppError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            Self::InvalidPackage(_) => (StatusCode::BAD_REQUEST, "INVALID_PACKAGE"),
            Self::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE"),
            Self::RateLimitExceeded => (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMIT_EXCEEDED"),
            Self::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            Self::Ipfs(_) => (StatusCode::BAD_GATEWAY, "IPFS_ERROR"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();

        // Don't leak internal details for 5xx
        let message = match status.is_server_error() {
            true => "an internal error occurred".to_string(),
            false => self.to_string(),
        };

        let body = ErrorBody {
            error: ErrorDetail { code, message },
            request_id: current_request_id().unwrap_or_else(|| format!("req_{}", Uuid::new_v4())),
        };

        (status, Json(body)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;

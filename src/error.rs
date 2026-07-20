use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::WWW_AUTHENTICATE},
    response::IntoResponse,
};
use serde::Serialize;
use serde_json::json;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AmosError>;

#[derive(Debug, Error)]
pub enum AmosError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unauthenticated: {0}")]
    Unauthenticated(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("required context role is unavailable: {0}")]
    RequiredRoleMissing(String),
    #[error("invalid state transition: {0}")]
    InvalidTransition(String),
    #[error("idempotency conflict: {0}")]
    IdempotencyConflict(String),
    #[error("capability rejected: {0}")]
    Capability(String),
    #[error("connector unavailable: {0}")]
    Connector(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("storage failure: {0}")]
    Storage(String),
    #[error("serialization failure: {0}")]
    Serialization(String),
}

impl From<rusqlite::Error> for AmosError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Storage(value.to_string())
    }
}

impl From<serde_json::Error> for AmosError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

impl From<std::io::Error> for AmosError {
    fn from(value: std::io::Error) -> Self {
        Self::Storage(value.to_string())
    }
}

#[derive(Serialize)]
struct ErrorEnvelope {
    request_id: String,
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    retryable: bool,
    review_required: bool,
    safe_details: serde_json::Value,
}

impl IntoResponse for AmosError {
    fn into_response(self) -> axum::response::Response {
        let requires_bearer_authentication = matches!(&self, Self::Unauthenticated(_));
        let (status, code, retryable, review_required) = match &self {
            Self::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND", false, false),
            Self::Unauthenticated(_) => (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED", false, false),
            Self::PermissionDenied(_) => (StatusCode::FORBIDDEN, "PERMISSION_DENIED", false, false),
            Self::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT", true, true),
            Self::Validation(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_FAILED",
                false,
                false,
            ),
            Self::RequiredRoleMissing(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "CONTEXT_REQUIRED_ROLE_MISSING",
                false,
                true,
            ),
            Self::InvalidTransition(_) => (
                StatusCode::CONFLICT,
                "ATXN_TRANSITION_CONFLICT",
                true,
                false,
            ),
            Self::IdempotencyConflict(_) => {
                (StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT", false, false)
            }
            Self::Capability(_) => (StatusCode::FORBIDDEN, "CAPABILITY_REJECTED", false, false),
            Self::Connector(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "CONNECTOR_UNAVAILABLE",
                true,
                false,
            ),
            Self::Execution(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "EXECUTION_FAILED",
                true,
                false,
            ),
            Self::Storage(_) | Self::Serialization(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                true,
                false,
            ),
        };
        let request_id = format!("req_{}", uuid::Uuid::new_v4().simple());
        let body = ErrorEnvelope {
            request_id: request_id.clone(),
            error: ErrorBody {
                code,
                message: self.to_string(),
                retryable,
                review_required,
                safe_details: json!({}),
            },
        };
        let mut response = (status, Json(body)).into_response();
        if requires_bearer_authentication {
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("x-request-id", value);
        }
        response
    }
}

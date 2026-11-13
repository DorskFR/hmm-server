use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    body: Value,
}

impl ApiError {
    #[must_use]
    pub fn new(status: StatusCode, message: &str) -> Self {
        Self { status, body: json!({ "error": message }) }
    }

    #[must_use]
    pub fn bad_request(message: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    #[must_use]
    pub fn invalid(message: &str, details: &[String]) -> Self {
        Self::bad_request(message).with("details", json!(details))
    }

    #[must_use]
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not found")
    }

    #[must_use]
    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "missing scope")
    }

    #[must_use]
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }

    #[must_use]
    pub fn with(mut self, field: &str, value: Value) -> Self {
        self.body[field] = value;
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = ?err, "database error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl From<object_store::Error> for ApiError {
    fn from(err: object_store::Error) -> Self {
        if matches!(err, object_store::Error::NotFound { .. }) {
            return Self::not_found();
        }
        tracing::error!(error = ?err, "object store error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

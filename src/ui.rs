use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::Method;
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

use crate::AppState;
use crate::error::ApiError;

/// Serves the built web UI, answering unknown paths with `index.html` so its client-side routes
/// load. A missing directory answers 404.
pub async fn serve(State(state): State<Arc<AppState>>, request: Request) -> Response {
    let path = request.uri().path();
    let api = path == "/v1" || path.starts_with("/v1/");
    if api || !matches!(*request.method(), Method::GET | Method::HEAD) {
        return ApiError::not_found().into_response();
    }
    let files =
        ServeDir::new(&state.ui_dir).fallback(ServeFile::new(state.ui_dir.join("index.html")));
    match files.oneshot(request).await {
        Ok(response) => response.into_response(),
        Err(never) => match never {},
    }
}

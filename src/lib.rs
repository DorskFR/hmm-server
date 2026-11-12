pub mod config;
pub mod envelope;
pub mod ingest;
pub mod policy;
pub mod redact;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderName, Method, StatusCode, header};
use axum::routing::{get, post};
use object_store::ObjectStore;
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub static MIGRATOR: Migrator = sqlx::migrate!();

pub struct AppState {
    pub db: PgPool,
    pub blobs: Arc<dyn ObjectStore>,
    pub policy: policy::Policy,
    pub limiter: policy::Limiter,
}

async fn healthz(State(state): State<Arc<AppState>>) -> StatusCode {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

pub fn build_router(state: Arc<AppState>, max_body_bytes: usize) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::POST])
        .allow_headers([header::CONTENT_TYPE, HeaderName::from_static(ingest::KEY_HEADER)]);
    Router::new()
        .route("/v1/reports", post(ingest::receive))
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .layer(cors)
        .route("/healthz", get(healthz))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

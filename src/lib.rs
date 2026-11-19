pub mod admin;
pub mod auth;
pub mod config;
pub mod envelope;
pub mod error;
pub mod events;
pub mod ingest;
pub mod policy;
pub mod redact;
pub mod ui;
pub mod webhook;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderName, Method, StatusCode, header};
use axum::routing::{get, post};
use object_store::ObjectStore;
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use tokio::sync::Notify;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub static MIGRATOR: Migrator = sqlx::migrate!();

const ACCESS_LOG_DAYS: i32 = 90;
const PRUNE_EVERY: Duration = Duration::from_secs(3600);

pub struct AppState {
    pub db: PgPool,
    pub blobs: Arc<dyn ObjectStore>,
    pub policy: policy::Policy,
    pub limiter: policy::Limiter,
    pub webhooks: webhook::Settings,
    /// Wakes the webhook worker once a delivery has been committed.
    pub wake: Notify,
    pub ui_dir: PathBuf,
}

impl AppState {
    pub fn new(db: PgPool, blobs: Arc<dyn ObjectStore>, policy: policy::Policy) -> Self {
        Self {
            db,
            blobs,
            policy,
            limiter: policy::Limiter::default(),
            webhooks: webhook::Settings::default(),
            wake: Notify::new(),
            ui_dir: PathBuf::new(),
        }
    }
}

async fn healthz(State(state): State<Arc<AppState>>) -> StatusCode {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// The public listener: ingest, plus the admin API for bearer keys only.
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
        .with_state(state.clone())
        .merge(admin::router(state, false))
        .layer(TraceLayer::new_for_http())
}

/// The admin listener: the admin API trusting the gateway's client certificate header, and the
/// web UI.
pub fn build_admin_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(ui::serve)
        .with_state(state.clone())
        .merge(admin::router(state, true))
        .layer(TraceLayer::new_for_http())
}

pub async fn prune_access_log(db: &PgPool) -> sqlx::Result<u64> {
    sqlx::query("DELETE FROM access_log WHERE at < now() - make_interval(days => $1)")
        .bind(ACCESS_LOG_DAYS)
        .execute(db)
        .await
        .map(|deleted| deleted.rows_affected())
}

pub async fn run_maintenance(state: Arc<AppState>) {
    let mut tick = tokio::time::interval(PRUNE_EVERY);
    loop {
        tick.tick().await;
        match prune_access_log(&state.db).await {
            Ok(0) => {}
            Ok(pruned) => tracing::info!(pruned, "access log pruned"),
            Err(err) => tracing::error!(error = ?err, "pruning the access log"),
        }
    }
}

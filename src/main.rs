use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use hmm_server::config::Config;
use hmm_server::policy::{Limiter, Policy};
use hmm_server::{AppState, MIGRATOR, build_router};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = Config::parse();
    let policy = Policy::parse(config.policy.as_deref()).context("HMM_POLICY is not valid")?;
    let db = PgPoolOptions::new()
        .max_connections(config.db_connections)
        .connect(&config.database_url)
        .await
        .context("connecting to Postgres")?;
    MIGRATOR.run(&db).await.context("applying migrations")?;
    let blobs = config.blobs().context("configuring the object store")?;
    let state = Arc::new(AppState { db, blobs, policy, limiter: Limiter::default() });

    let listener = TcpListener::bind(&config.bind).await.context("binding")?;
    tracing::info!(bind = %config.bind, "listening");
    let mut terminate = signal(SignalKind::terminate())?;
    axum::serve(listener, build_router(state, config.max_body_bytes))
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        })
        .await?;
    Ok(())
}

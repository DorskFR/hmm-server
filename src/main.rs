use std::sync::Arc;

use anyhow::{Context, bail};
use chrono::{Duration, Utc};
use clap::Parser;
use hmm_server::config::{Cli, Command, Config, KeyCommand};
use hmm_server::policy::Policy;
use hmm_server::{
    AppState, MIGRATOR, auth, build_admin_router, build_router, run_maintenance, webhook,
};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;

async fn database(config: &Config) -> anyhow::Result<PgPool> {
    let db = PgPoolOptions::new()
        .max_connections(config.db_connections)
        .connect(config.database_url()?)
        .await
        .context("connecting to Postgres")?;
    MIGRATOR.run(&db).await.context("applying migrations")?;
    Ok(db)
}

async fn serve(config: Config) -> anyhow::Result<()> {
    let policy = Policy::parse(config.policy.as_deref()).context("HMM_POLICY is not valid")?;
    let blobs = config.blobs().context("configuring the object store")?;
    let db = database(&config).await?;
    let state = Arc::new(AppState {
        webhooks: webhook::Settings::allowing(&config.webhook_allow_hosts),
        ui_dir: config.ui_dir.clone(),
        ..AppState::new(db, blobs, policy)
    });
    if !state.ui_dir.join("index.html").is_file() {
        tracing::warn!(dir = %state.ui_dir.display(), "no web UI found; / will answer 404");
    }
    tokio::spawn(run_maintenance(state.clone()));

    let public = TcpListener::bind(&config.bind).await.context("binding HMM_BIND")?;
    let admin = TcpListener::bind(&config.admin_bind).await.context("binding HMM_ADMIN_BIND")?;
    tracing::info!(bind = %config.bind, admin_bind = %config.admin_bind, "listening");

    let (stop, stopped) = watch::channel(());
    let until_stopped = |mut stopped: watch::Receiver<()>| async move {
        let _ = stopped.changed().await;
    };
    let deliveries = tokio::spawn(webhook::run(state.clone(), stopped.clone()));
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::spawn(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        drop(stop);
    });
    let public = axum::serve(public, build_router(state.clone(), config.max_body_bytes))
        .with_graceful_shutdown(until_stopped(stopped.clone()));
    let admin = axum::serve(admin, build_admin_router(state))
        .with_graceful_shutdown(until_stopped(stopped));
    tokio::try_join!(public.into_future(), admin.into_future())?;
    deliveries.await?;
    Ok(())
}

async fn key(config: &Config, command: KeyCommand) -> anyhow::Result<()> {
    let db = database(config).await?;
    match command {
        KeyCommand::Create { name, projects, scopes, expires_in_days } => {
            let problems = auth::validate_key(&name, &projects, &scopes);
            if !problems.is_empty() {
                bail!(problems.join("; "));
            }
            let expires_at = expires_in_days.map(|days| Utc::now() + Duration::days(days.into()));
            let (key, token) = auth::create_key(&db, &name, &projects, &scopes, expires_at)
                .await
                .map_err(|err| {
                if auth::is_unique_violation(&err) {
                    anyhow::anyhow!("a live key is already named {name}")
                } else {
                    err.into()
                }
            })?;
            eprintln!("Created key {} ({}). The token is shown only now:", key.name, key.prefix);
            println!("{token}");
        }
        KeyCommand::Ls => {
            let keys: Vec<auth::Key> = sqlx::query_as(
                "SELECT id, name, prefix, projects, scopes, created_at, expires_at, revoked_at, \
                 last_used_at FROM api_keys ORDER BY created_at",
            )
            .fetch_all(&db)
            .await?;
            let day = |at: Option<chrono::DateTime<Utc>>| {
                at.map_or_else(|| "-".to_owned(), |at| at.format("%Y-%m-%d").to_string())
            };
            println!(
                "{:<10}{:<24}{:<12}{:<12}{:<12}{:<12}{:<24}SCOPES",
                "PREFIX", "NAME", "CREATED", "EXPIRES", "REVOKED", "LAST USED", "PROJECTS"
            );
            for key in keys {
                println!(
                    "{:<10}{:<24}{:<12}{:<12}{:<12}{:<12}{:<24}{}",
                    key.prefix,
                    key.name,
                    day(Some(key.created_at)),
                    day(key.expires_at),
                    day(key.revoked_at),
                    day(key.last_used_at),
                    key.projects.join(","),
                    key.scopes.join(","),
                );
            }
        }
        KeyCommand::Revoke { prefix } => {
            let revoked: Option<String> = sqlx::query_scalar(
                "UPDATE api_keys SET revoked_at = now() \
                 WHERE prefix = $1 AND revoked_at IS NULL RETURNING name",
            )
            .bind(&prefix)
            .fetch_optional(&db)
            .await?;
            match revoked {
                Some(name) => println!("Revoked {name} ({prefix})."),
                None => bail!("no live key has the prefix {prefix}"),
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    match cli.command {
        None | Some(Command::Serve) => serve(cli.config).await,
        Some(Command::Key { command }) => key(&cli.config, command).await,
    }
}

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{body, cursor, id, limit, page, param, params};
use crate::auth::{self, Caller};
use crate::error::ApiError;
use crate::{AppState, webhook};

macro_rules! select_webhooks {
    ($tail:literal) => {
        concat!(
            "SELECT w.id, w.name, w.project, w.url, w.events, w.enabled, w.created_at, \
             d.updated_at AS last_at, d.last_status_code, d.last_error \
             FROM webhooks w LEFT JOIN LATERAL ( \
               SELECT updated_at, last_status_code, last_error FROM webhook_deliveries \
               WHERE webhook_id = w.id AND last_duration_ms IS NOT NULL \
               ORDER BY updated_at DESC LIMIT 1 \
             ) d ON true ",
            $tail
        )
    };
}

#[derive(sqlx::FromRow)]
struct Webhook {
    id: Uuid,
    name: String,
    project: String,
    url: String,
    events: Vec<String>,
    enabled: bool,
    created_at: DateTime<Utc>,
    last_at: Option<DateTime<Utc>>,
    last_status_code: Option<i32>,
    last_error: Option<String>,
}

impl Webhook {
    fn json(&self) -> Value {
        let last_delivery = self.last_at.map(|at| {
            json!({ "at": at, "status_code": self.last_status_code, "error": self.last_error })
        });
        json!({
            "id": self.id,
            "name": self.name,
            "project": self.project,
            "url": self.url,
            "events": self.events,
            "enabled": self.enabled,
            "created_at": self.created_at,
            "last_delivery": last_delivery,
        })
    }
}

async fn find(db: &PgPool, caller: &Caller, webhook: &str) -> Result<Webhook, ApiError> {
    let found: Option<Webhook> = sqlx::query_as(select_webhooks!("WHERE w.id = $1"))
        .bind(id(webhook)?)
        .fetch_optional(db)
        .await?;
    found.filter(|webhook| caller.sees(&webhook.project)).ok_or_else(ApiError::not_found)
}

fn name_problem(name: &str) -> Option<String> {
    (name.trim().is_empty() || name.chars().count() > 100)
        .then(|| "name must be 1 to 100 characters".to_owned())
}

fn events_problem(events: &[String]) -> Option<String> {
    (events.is_empty() || events.iter().any(|e| !webhook::SUBSCRIBABLE.contains(&e.as_str())))
        .then(|| format!("events must be some of {}", webhook::SUBSCRIBABLE.join(", ")))
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::ADMIN)?;
    let rows: Vec<Webhook> = sqlx::query_as(select_webhooks!(
        "WHERE ($1::text[] IS NULL OR w.project = ANY($1)) ORDER BY w.created_at DESC, w.id"
    ))
    .bind(caller.visible())
    .fetch_all(&state.db)
    .await?;
    let webhooks: Vec<Value> = rows.iter().map(Webhook::json).collect();
    Ok(Json(json!({ "webhooks": webhooks })))
}

#[derive(Deserialize)]
struct NewWebhook {
    name: String,
    project: String,
    url: String,
    events: Vec<String>,
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    caller.require(auth::ADMIN)?;
    let new: NewWebhook = body(&bytes)?;
    let mut problems: Vec<String> =
        name_problem(&new.name).into_iter().chain(events_problem(&new.events)).collect();
    if !auth::is_project_name(&new.project) {
        problems.push("project is not a project name".to_owned());
    }
    if let Err(problem) = state.webhooks.check_url(&new.url).await {
        problems.push(problem);
    }
    if !problems.is_empty() {
        return Err(ApiError::invalid("invalid webhook", &problems));
    }
    if !caller.sees(&new.project) {
        return Err(ApiError::not_found());
    }
    let secret = format!("whsec_{}", auth::random_alnum(40));
    let created: Uuid = sqlx::query_scalar(
        "INSERT INTO webhooks (name, project, url, events, secret) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(new.name.trim())
    .bind(&new.project)
    .bind(&new.url)
    .bind(&new.events)
    .bind(&secret)
    .fetch_one(&state.db)
    .await?;
    tracing::info!(actor = %caller.actor, webhook = %created, "webhook created");
    let mut webhook = find(&state.db, &caller, &created.to_string()).await?.json();
    webhook["secret"] = json!(secret);
    Ok((StatusCode::CREATED, Json(webhook)))
}

#[derive(Deserialize)]
struct Changes {
    name: Option<String>,
    url: Option<String>,
    events: Option<Vec<String>>,
    enabled: Option<bool>,
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path(webhook): Path<String>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::ADMIN)?;
    let current = find(&state.db, &caller, &webhook).await?;
    let changes: Changes = body(&bytes)?;
    let mut problems: Vec<String> = changes
        .name
        .as_deref()
        .and_then(name_problem)
        .into_iter()
        .chain(changes.events.as_deref().and_then(events_problem))
        .collect();
    if let Some(url) = &changes.url
        && let Err(problem) = state.webhooks.check_url(url).await
    {
        problems.push(problem);
    }
    if !problems.is_empty() {
        return Err(ApiError::invalid("invalid webhook", &problems));
    }
    sqlx::query(
        "UPDATE webhooks SET name = COALESCE($2, name), url = COALESCE($3, url), \
         events = COALESCE($4, events), enabled = COALESCE($5, enabled) WHERE id = $1",
    )
    .bind(current.id)
    .bind(changes.name.as_deref().map(str::trim))
    .bind(changes.url)
    .bind(changes.events)
    .bind(changes.enabled)
    .execute(&state.db)
    .await?;
    Ok(Json(find(&state.db, &caller, &webhook).await?.json()))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path(webhook): Path<String>,
) -> Result<StatusCode, ApiError> {
    caller.require(auth::ADMIN)?;
    let current = find(&state.db, &caller, &webhook).await?;
    sqlx::query("DELETE FROM webhooks WHERE id = $1").bind(current.id).execute(&state.db).await?;
    tracing::info!(actor = %caller.actor, webhook = %current.id, "webhook deleted");
    Ok(StatusCode::NO_CONTENT)
}

macro_rules! select_deliveries {
    ($tail:literal) => {
        concat!(
            "SELECT d.id, d.webhook_id, w.name AS webhook_name, d.event, d.project, d.report_id, \
             d.state, d.attempts, \
             CASE WHEN d.state = 'pending' THEN d.next_attempt_at END AS next_attempt_at, \
             d.last_status_code, d.last_error, d.last_duration_ms, d.created_at, d.updated_at \
             FROM webhook_deliveries d JOIN webhooks w ON w.id = d.webhook_id ",
            $tail
        )
    };
}

#[derive(Serialize, sqlx::FromRow)]
pub(super) struct Delivery {
    id: Uuid,
    webhook_id: Uuid,
    webhook_name: String,
    event: String,
    project: String,
    report_id: Option<String>,
    state: String,
    attempts: i32,
    next_attempt_at: Option<DateTime<Utc>>,
    last_status_code: Option<i32>,
    last_error: Option<String>,
    last_duration_ms: Option<i32>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

async fn delivery(db: &PgPool, id: Uuid) -> Result<Delivery, ApiError> {
    Ok(sqlx::query_as(select_deliveries!("WHERE d.id = $1")).bind(id).fetch_one(db).await?)
}

pub async fn ping(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path(webhook): Path<String>,
) -> Result<(StatusCode, Json<Delivery>), ApiError> {
    caller.require(auth::ADMIN)?;
    let hook = find(&state.db, &caller, &webhook).await?;
    let payload = json!({ "event": webhook::PING, "at": Utc::now(), "project": hook.project });
    let queued: Uuid = sqlx::query_scalar(
        "INSERT INTO webhook_deliveries (webhook_id, event, project, payload) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(hook.id)
    .bind(webhook::PING)
    .bind(&hook.project)
    .bind(payload)
    .fetch_one(&state.db)
    .await?;
    state.wake.notify_one();
    Ok((StatusCode::ACCEPTED, Json(delivery(&state.db, queued).await?)))
}

pub async fn deliveries(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    RawQuery(query): RawQuery,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::ADMIN)?;
    let params = params(query.as_deref());
    let limit = limit(&params)?;
    let webhook = param(&params, "webhook")
        .map(|text| Uuid::parse_str(text).map_err(|_| ApiError::bad_request("invalid webhook")))
        .transpose()?;
    let (before_at, before_id) = cursor::<(DateTime<Utc>, Uuid)>(&params)?.unzip();
    let rows: Vec<Delivery> = sqlx::query_as(select_deliveries!(
        "WHERE ($1::uuid IS NULL OR d.webhook_id = $1) \
         AND ($2::text[] IS NULL OR d.project = ANY($2)) \
         AND ($3::timestamptz IS NULL OR (d.created_at, d.id) < ($3, $4)) \
         ORDER BY d.created_at DESC, d.id DESC LIMIT $5"
    ))
    .bind(webhook)
    .bind(caller.visible())
    .bind(before_at)
    .bind(before_id)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;
    let (deliveries, next_cursor) = page(rows, limit, |row| (row.created_at, row.id));
    Ok(Json(json!({ "deliveries": deliveries, "next_cursor": next_cursor })))
}

pub async fn redeliver(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path(delivery_id): Path<String>,
) -> Result<(StatusCode, Json<Delivery>), ApiError> {
    caller.require(auth::ADMIN)?;
    let delivery_id = id(&delivery_id)?;
    let project: Option<String> =
        sqlx::query_scalar("SELECT project FROM webhook_deliveries WHERE id = $1")
            .bind(delivery_id)
            .fetch_optional(&state.db)
            .await?;
    project.filter(|project| caller.sees(project)).ok_or_else(ApiError::not_found)?;
    sqlx::query(
        "UPDATE webhook_deliveries SET state = 'pending', next_attempt_at = now(), \
         updated_at = now() WHERE id = $1",
    )
    .bind(delivery_id)
    .execute(&state.db)
    .await?;
    state.wake.notify_one();
    Ok((StatusCode::ACCEPTED, Json(delivery(&state.db, delivery_id).await?)))
}

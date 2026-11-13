use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use object_store::ObjectStoreExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{PgPool, Postgres, QueryBuilder};

use super::{body, cursor, limit, optional_body, page, param, params};
use crate::auth::{self, Caller};
use crate::error::ApiError;
use crate::events::{self, Event};
use crate::{AppState, webhook};

const STATUSES: [&str; 5] = ["new", "triaged", "in_progress", "resolved", "dismissed"];
const OPEN: [&str; 3] = ["new", "triaged", "in_progress"];
const DEFAULT_LEASE_SECONDS: i64 = 1800;

macro_rules! select_reports {
    ($tail:literal) => {
        concat!(
            "SELECT r.project, r.id, r.kind, r.category, r.comment, r.page_url, r.client_name, \
             r.client_version, r.created_at, r.received_at, r.status, r.assignee, r.ticket, \
             r.claimed_by, r.claimed_until, (r.claimed_until > now()) IS TRUE AS claim_live, \
             (SELECT count(*) FROM attachments a \
              WHERE a.project = r.project AND a.report_id = r.id) AS attachment_count, \
             r.origin, r.remote_addr \
             FROM reports r ",
            $tail
        )
    };
}

#[derive(sqlx::FromRow)]
struct Report {
    project: String,
    id: String,
    kind: String,
    category: Option<String>,
    comment: String,
    page_url: Option<String>,
    client_name: String,
    client_version: String,
    created_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
    status: String,
    assignee: Option<String>,
    ticket: Option<String>,
    claimed_by: Option<String>,
    claimed_until: Option<DateTime<Utc>>,
    claim_live: bool,
    attachment_count: i64,
    origin: Option<String>,
    remote_addr: Option<String>,
}

impl Report {
    fn claim(&self) -> Value {
        if self.claim_live {
            json!({ "by": self.claimed_by, "until": self.claimed_until })
        } else {
            Value::Null
        }
    }

    fn summary(&self) -> Value {
        json!({
            "project": self.project,
            "id": self.id,
            "kind": self.kind,
            "category": self.category,
            "comment": self.comment,
            "page_url": self.page_url,
            "client_name": self.client_name,
            "client_version": self.client_version,
            "created_at": self.created_at,
            "received_at": self.received_at,
            "status": self.status,
            "assignee": self.assignee,
            "ticket": self.ticket,
            "claim": self.claim(),
            "attachments": self.attachment_count,
        })
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct Attachment {
    id: String,
    mime: String,
    width: i32,
    height: i32,
    bytes: i64,
}

/// A report outside the caller's projects does not exist as far as the caller can tell.
fn visible(caller: &Caller, project: &str) -> Result<(), ApiError> {
    if caller.sees(project) { Ok(()) } else { Err(ApiError::not_found()) }
}

async fn detail(db: &PgPool, caller: &Caller, project: &str, id: &str) -> Result<Value, ApiError> {
    let report: Report = sqlx::query_as(select_reports!("WHERE r.project = $1 AND r.id = $2"))
        .bind(project)
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let envelope: Value =
        sqlx::query_scalar("SELECT envelope FROM reports WHERE project = $1 AND id = $2")
            .bind(project)
            .bind(id)
            .fetch_one(db)
            .await?;
    let attachments: Vec<Attachment> = sqlx::query_as(
        "SELECT id, mime, width, height, bytes FROM attachments \
         WHERE project = $1 AND report_id = $2 ORDER BY id",
    )
    .bind(project)
    .bind(id)
    .fetch_all(db)
    .await?;
    let mut body = report.summary();
    body["origin"] = json!(report.origin);
    if caller.has(auth::ADMIN) {
        body["remote_addr"] = json!(report.remote_addr);
    }
    body["envelope"] = envelope;
    body["attachments"] = json!(attachments);
    body["events"] = json!(events::of_report(db, project, id).await?);
    Ok(body)
}

fn statuses(params: &[(String, String)]) -> Result<Vec<String>, ApiError> {
    let mut wanted = Vec::new();
    let values = params.iter().filter(|(key, _)| key == "status");
    for status in values.flat_map(|(_, value)| value.split(',')).map(str::trim) {
        if status == "open" {
            wanted.extend(OPEN.map(str::to_owned));
        } else if STATUSES.contains(&status) {
            wanted.push(status.to_owned());
        } else {
            return Err(ApiError::bad_request("unknown status"));
        }
    }
    Ok(wanted)
}

type Position = (DateTime<Utc>, String, String);

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    RawQuery(query): RawQuery,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::READ)?;
    let params = params(query.as_deref());
    let limit = limit(&params)?;
    let statuses = statuses(&params)?;
    let since = param(&params, "since")
        .map(|text| {
            DateTime::parse_from_rfc3339(text)
                .map(|at| at.with_timezone(&Utc))
                .map_err(|_| ApiError::bad_request("since must be an RFC 3339 timestamp"))
        })
        .transpose()?;
    let after: Option<Position> = cursor(&params)?;

    let mut sql = QueryBuilder::<Postgres>::new(select_reports!("WHERE true"));
    if let Some(projects) = caller.visible() {
        sql.push(" AND r.project = ANY(").push_bind(projects).push(")");
    }
    if let Some(project) = param(&params, "project") {
        sql.push(" AND r.project = ").push_bind(project);
    }
    if !statuses.is_empty() {
        sql.push(" AND r.status = ANY(").push_bind(statuses).push(")");
    }
    if let Some(kind) = param(&params, "kind") {
        sql.push(" AND r.kind = ").push_bind(kind);
    }
    if let Some(q) = param(&params, "q") {
        sql.push(" AND position(lower(")
            .push_bind(q)
            .push(") in lower(r.comment || ' ' || COALESCE(r.page_url, '') || ' ' || r.id)) > 0");
    }
    if let Some(since) = since {
        sql.push(" AND r.received_at > ").push_bind(since);
    }
    let (comparison, direction) = if since.is_some() { (">", "") } else { ("<", "DESC") };
    if let Some((received_at, project, id)) = after {
        sql.push(format!(" AND (r.received_at, r.project, r.id) {comparison} ("))
            .push_bind(received_at)
            .push(", ")
            .push_bind(project)
            .push(", ")
            .push_bind(id)
            .push(")");
    }
    sql.push(format!(
        " ORDER BY r.received_at {direction}, r.project {direction}, r.id {direction} LIMIT "
    ))
    .push_bind(limit + 1);

    let rows: Vec<Report> = sql.build_query_as().fetch_all(&state.db).await?;
    let (rows, next_cursor) =
        page(rows, limit, |row| (row.received_at, row.project.clone(), row.id.clone()));
    let reports: Vec<Value> = rows.iter().map(Report::summary).collect();
    Ok(Json(json!({ "reports": reports, "next_cursor": next_cursor })))
}

pub async fn projects(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::READ)?;
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT project, status, count(*) FROM reports \
         WHERE ($1::text[] IS NULL OR project = ANY($1)) GROUP BY project, status",
    )
    .bind(caller.visible())
    .fetch_all(&state.db)
    .await?;
    let empty = || STATUSES.iter().map(|&status| (status.to_owned(), 0)).collect();
    let mut counts: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for project in state.policy.projects.iter().flat_map(|projects| projects.keys()) {
        if caller.sees(project) {
            counts.insert(project.clone(), empty());
        }
    }
    for (project, status, count) in rows {
        counts.entry(project).or_insert_with(empty).insert(status, count);
    }
    let projects: Vec<Value> = counts
        .into_iter()
        .map(|(project, counts)| json!({ "project": project, "counts": counts }))
        .collect();
    Ok(Json(json!({ "projects": projects })))
}

pub async fn show(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path((project, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::READ)?;
    visible(&caller, &project)?;
    Ok(Json(detail(&state.db, &caller, &project, &id).await?))
}

pub async fn attachment(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path((project, id, attachment)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    caller.require(auth::ATTACHMENTS)?;
    visible(&caller, &project)?;
    let found: Option<(String, String)> = sqlx::query_as(
        "SELECT object_key, mime FROM attachments \
         WHERE project = $1 AND report_id = $2 AND id = $3",
    )
    .bind(&project)
    .bind(&id)
    .bind(&attachment)
    .fetch_optional(&state.db)
    .await?;
    let (key, mime) = found.ok_or_else(ApiError::not_found)?;
    let object = state.blobs.get(&object_store::path::Path::from(key)).await?;
    let headers = [
        (header::CONTENT_TYPE, mime),
        (header::CONTENT_LENGTH, object.meta.size.to_string()),
        (header::CACHE_CONTROL, "private, max-age=3600".to_owned()),
    ];
    Ok((headers, Body::from_stream(object.into_stream())).into_response())
}

/// A field that is absent (`None`), cleared (`Some(None)`) or set.
#[allow(clippy::option_option)]
fn text_change(patch: &Map<String, Value>, name: &str) -> Result<Option<Option<String>>, ApiError> {
    match patch.get(name) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(text)) if text.chars().count() <= 200 => {
            let text = text.trim();
            Ok(Some((!text.is_empty()).then(|| text.to_owned())))
        }
        Some(_) => Err(ApiError::bad_request(&format!(
            "{name} must be null or a string of at most 200 characters"
        ))),
    }
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path((project, id)): Path<(String, String)>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::TRIAGE)?;
    visible(&caller, &project)?;
    let patch: Map<String, Value> = body(&bytes)?;
    let status = match patch.get("status") {
        None => None,
        Some(Value::String(status)) if STATUSES.contains(&status.as_str()) => Some(status.clone()),
        Some(_) => {
            return Err(ApiError::bad_request(&format!(
                "status must be one of {}",
                STATUSES.join(", ")
            )));
        }
    };
    let assignee = text_change(&patch, "assignee")?;
    let ticket = text_change(&patch, "ticket")?;

    let mut tx = state.db.begin().await?;
    let current: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT status, assignee, ticket FROM reports WHERE project = $1 AND id = $2 FOR UPDATE",
    )
    .bind(&project)
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (old_status, old_assignee, old_ticket) = current.ok_or_else(ApiError::not_found)?;
    let mut queued = 0;
    if let Some(status) = status.filter(|status| *status != old_status) {
        sqlx::query("UPDATE reports SET status = $3 WHERE project = $1 AND id = $2")
            .bind(&project)
            .bind(&id)
            .bind(&status)
            .execute(&mut *tx)
            .await?;
        let change = json!({ "from": old_status, "to": status });
        events::record(&mut *tx, &project, &id, &caller.actor, "status", change.clone()).await?;
        queued = webhook::enqueue(&mut tx, &project, &id, webhook::STATUS_CHANGED, change).await?;
    }
    let texts = [
        ("assignee", "assigned", assignee, old_assignee),
        ("ticket", "ticket", ticket, old_ticket),
    ];
    for (column, event, new, old) in texts {
        let Some(new) = new.filter(|new| *new != old) else { continue };
        let sql = match column {
            "assignee" => "UPDATE reports SET assignee = $3 WHERE project = $1 AND id = $2",
            _ => "UPDATE reports SET ticket = $3 WHERE project = $1 AND id = $2",
        };
        sqlx::query(sql).bind(&project).bind(&id).bind(&new).execute(&mut *tx).await?;
        let change = json!({ "from": old, "to": new });
        events::record(&mut *tx, &project, &id, &caller.actor, event, change).await?;
    }
    tx.commit().await?;
    if queued > 0 {
        state.wake.notify_one();
    }
    Ok(Json(detail(&state.db, &caller, &project, &id).await?))
}

#[derive(sqlx::FromRow)]
struct Claim {
    claimed_by: Option<String>,
    claimed_until: Option<DateTime<Utc>>,
    live: bool,
}

impl Claim {
    fn held_by_another(&self, caller: &Caller) -> bool {
        self.live && self.claimed_by.as_deref() != Some(caller.actor.as_str())
    }

    fn conflict(&self) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, "claimed by someone else")
            .with("claim", json!({ "by": self.claimed_by, "until": self.claimed_until }))
    }
}

async fn lock_claim(
    tx: &mut sqlx::PgConnection,
    project: &str,
    id: &str,
) -> Result<Claim, ApiError> {
    sqlx::query_as(
        "SELECT claimed_by, claimed_until, (claimed_until > now()) IS TRUE AS live \
         FROM reports WHERE project = $1 AND id = $2 FOR UPDATE",
    )
    .bind(project)
    .bind(id)
    .fetch_optional(tx)
    .await?
    .ok_or_else(ApiError::not_found)
}

#[derive(Default, Deserialize)]
struct Lease {
    lease_seconds: Option<i64>,
}

pub async fn claim(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path((project, id)): Path<(String, String)>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::TRIAGE)?;
    visible(&caller, &project)?;
    let seconds = optional_body::<Lease>(&bytes)?.lease_seconds.unwrap_or(DEFAULT_LEASE_SECONDS);
    if !(60..=86400).contains(&seconds) {
        return Err(ApiError::bad_request("lease_seconds must be between 60 and 86400"));
    }
    let mut tx = state.db.begin().await?;
    let current = lock_claim(&mut tx, &project, &id).await?;
    if current.held_by_another(&caller) {
        return Err(current.conflict());
    }
    let until: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE reports SET claimed_by = $3, \
         claimed_until = now() + make_interval(secs => $4::double precision) \
         WHERE project = $1 AND id = $2 RETURNING claimed_until",
    )
    .bind(&project)
    .bind(&id)
    .bind(&caller.actor)
    .bind(seconds)
    .fetch_one(&mut *tx)
    .await?;
    events::record(&mut *tx, &project, &id, &caller.actor, "claimed", json!({ "until": until }))
        .await?;
    tx.commit().await?;
    Ok(Json(detail(&state.db, &caller, &project, &id).await?))
}

pub async fn release(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path((project, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    caller.require(auth::TRIAGE)?;
    visible(&caller, &project)?;
    let mut tx = state.db.begin().await?;
    let current = lock_claim(&mut tx, &project, &id).await?;
    if !current.live {
        return Ok(StatusCode::NO_CONTENT);
    }
    if current.held_by_another(&caller) && !caller.has(auth::ADMIN) {
        return Err(current.conflict());
    }
    sqlx::query(
        "UPDATE reports SET claimed_by = NULL, claimed_until = NULL WHERE project = $1 AND id = $2",
    )
    .bind(&project)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    events::record(&mut *tx, &project, &id, &caller.actor, "released", json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Note {
    note: String,
}

pub async fn note(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path((project, id)): Path<(String, String)>,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Event>), ApiError> {
    caller.require(auth::TRIAGE)?;
    visible(&caller, &project)?;
    let Note { note } = body(&bytes)?;
    if note.trim().is_empty() || note.chars().count() > 4000 {
        return Err(ApiError::bad_request("note must be 1 to 4000 characters"));
    }
    let known: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM reports WHERE project = $1 AND id = $2")
            .bind(&project)
            .bind(&id)
            .fetch_optional(&state.db)
            .await?;
    known.ok_or_else(ApiError::not_found)?;
    let event =
        events::record(&state.db, &project, &id, &caller.actor, "note", json!({ "note": note }))
            .await?;
    Ok((StatusCode::CREATED, Json(event)))
}

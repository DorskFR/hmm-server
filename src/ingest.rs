use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use object_store::path::Path;
use object_store::{ObjectStoreExt, PutPayload};
use serde_json::{Value, json};
use sqlx::types::Json as Jsonb;

use crate::policy::Denied;
use crate::{AppState, envelope, events, redact, webhook};

pub const KEY_HEADER: &str = "x-hmm-key";
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

pub struct Reject {
    status: StatusCode,
    error: &'static str,
    details: Vec<String>,
}

impl Reject {
    const fn new(status: StatusCode, error: &'static str) -> Self {
        Self { status, error, details: Vec::new() }
    }
}

impl IntoResponse for Reject {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.error });
        if !self.details.is_empty() {
            body["details"] = json!(self.details);
        }
        (self.status, Json(body)).into_response()
    }
}

fn internal(context: &'static str) -> impl Fn(anyhow::Error) -> Reject {
    move |err| {
        tracing::error!(error = ?err, "{context}");
        Reject::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty())
}

struct Blob {
    id: String,
    width: i64,
    height: i64,
    key: String,
    data: Vec<u8>,
}

fn blobs_of(report: &mut Value, project: &str, id: &str) -> Result<Vec<Blob>, Reject> {
    let mut blobs = Vec::new();
    for attachment in report["attachments"].as_array_mut().into_iter().flatten() {
        let data = attachment["data"].take();
        let data = STANDARD
            .decode(data.as_str().unwrap_or_default())
            .ok()
            .filter(|bytes| bytes.starts_with(PNG_MAGIC))
            .ok_or(Reject::new(StatusCode::BAD_REQUEST, "an attachment is not a PNG image"))?;
        let attachment_id = attachment["id"].as_str().unwrap_or_default().to_owned();
        let key = format!("{project}/{id}/{attachment_id}.png");
        let entry = attachment.as_object_mut().expect("validated as an object");
        entry.remove("data");
        entry.insert("bytes".into(), json!(data.len()));
        entry.insert("key".into(), json!(key));
        blobs.push(Blob {
            id: attachment_id,
            width: attachment["width"].as_i64().unwrap_or_default(),
            height: attachment["height"].as_i64().unwrap_or_default(),
            key,
            data,
        });
    }
    Ok(blobs)
}

async fn store(
    db: &sqlx::PgPool,
    report: &Value,
    blobs: &[Blob],
    origin: Option<&str>,
) -> anyhow::Result<u64> {
    let project = report["project"].as_str().unwrap_or_default();
    let id = report["id"].as_str().unwrap_or_default();
    let mut tx = db.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO reports (project, id, kind, category, fingerprint, comment, page_url, \
         client_name, client_version, created_at, origin, envelope) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::timestamptz, $11, $12) \
         ON CONFLICT DO NOTHING",
    )
    .bind(project)
    .bind(id)
    .bind(report["kind"].as_str())
    .bind(report["category"].as_str())
    .bind(report["fingerprint"].as_str())
    .bind(report["comment"].as_str())
    .bind(report["page"]["url"].as_str())
    .bind(report["client"]["name"].as_str())
    .bind(report["client"]["version"].as_str())
    .bind(report["createdAt"].as_str())
    .bind(origin)
    .bind(Jsonb(report))
    .execute(&mut *tx)
    .await?;
    let mut queued = 0;
    if inserted.rows_affected() == 1 {
        for blob in blobs {
            sqlx::query(
                "INSERT INTO attachments (project, report_id, id, mime, width, height, bytes, object_key) \
                 VALUES ($1, $2, $3, 'image/png', $4, $5, $6, $7)",
            )
            .bind(project)
            .bind(id)
            .bind(&blob.id)
            .bind(i32::try_from(blob.width).unwrap_or(i32::MAX))
            .bind(i32::try_from(blob.height).unwrap_or(i32::MAX))
            .bind(i64::try_from(blob.data.len()).unwrap_or(i64::MAX))
            .bind(&blob.key)
            .execute(&mut *tx)
            .await?;
        }
        let bytes: usize = blobs.iter().map(|blob| blob.data.len()).sum();
        let received = json!({ "attachments": blobs.len(), "bytes": bytes });
        events::record(&mut *tx, project, id, events::SYSTEM, "received", received).await?;
        queued = webhook::enqueue(&mut tx, project, id, webhook::CREATED, json!({})).await?;
    }
    tx.commit().await?;
    Ok(queued)
}

pub async fn receive(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Reject> {
    let key = header_str(&headers, KEY_HEADER)
        .ok_or(Reject::new(StatusCode::UNAUTHORIZED, "missing x-hmm-key"))?;
    let mut report: Value = serde_json::from_slice(&body)
        .map_err(|_| Reject::new(StatusCode::BAD_REQUEST, "invalid body"))?;
    envelope::validate(&report).map_err(|details| Reject {
        status: StatusCode::BAD_REQUEST,
        error: "invalid envelope",
        details,
    })?;
    let project = report["project"].as_str().unwrap_or_default().to_owned();
    let id = report["id"].as_str().unwrap_or_default().to_owned();
    if project != key {
        return Err(Reject::new(StatusCode::BAD_REQUEST, "project key mismatch"));
    }

    let origin = header_str(&headers, header::ORIGIN.as_str());
    let address = header_str(&headers, "x-forwarded-for")
        .and_then(|list| list.rsplit(',').next())
        .map(str::trim);
    state.policy.admit(&project, origin).map_err(|denied| {
        Reject::new(
            StatusCode::FORBIDDEN,
            match denied {
                Denied::UnknownProject => "unknown project",
                Denied::Origin => "origin not allowed",
            },
        )
    })?;
    if !state.limiter.allow(state.policy.rate_per_minute, &project, address.unwrap_or("-")) {
        return Err(Reject::new(StatusCode::TOO_MANY_REQUESTS, "too many reports"));
    }

    let known: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM reports WHERE project = $1 AND id = $2")
            .bind(&project)
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| internal("looking up the report")(e.into()))?;
    if known.is_some() {
        return Ok((StatusCode::OK, Json(json!({ "id": id, "duplicate": true }))).into_response());
    }

    if let Some(budget) = state.policy.storage_budget_bytes {
        let used: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(bytes), 0)::bigint FROM attachments")
                .fetch_one(&state.db)
                .await
                .map_err(|e| internal("measuring storage")(e.into()))?;
        if u64::try_from(used).unwrap_or(u64::MAX) >= budget {
            return Err(Reject::new(StatusCode::INSUFFICIENT_STORAGE, "storage budget reached"));
        }
    }

    redact::envelope(&mut report);
    let blobs = blobs_of(&mut report, &project, &id)?;
    for blob in &blobs {
        state
            .blobs
            .put(&Path::from(blob.key.as_str()), PutPayload::from(blob.data.clone()))
            .await
            .map_err(|e| internal("storing an attachment")(e.into()))?;
    }

    let queued =
        store(&state.db, &report, &blobs, origin).await.map_err(internal("storing the report"))?;
    if queued > 0 {
        state.wake.notify_one();
    }

    tracing::info!(%project, %id, attachments = blobs.len(), "report stored");
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))).into_response())
}

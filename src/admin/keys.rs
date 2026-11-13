use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{body, cursor, id, limit, page, param, params};
use crate::AppState;
use crate::auth::{self, Caller, Key};
use crate::error::ApiError;

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::ADMIN)?;
    let keys: Vec<Key> = sqlx::query_as(
        "SELECT id, name, prefix, projects, scopes, created_at, expires_at, revoked_at, \
         last_used_at FROM api_keys ORDER BY created_at DESC, id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "keys": keys })))
}

#[derive(Deserialize)]
struct NewKey {
    name: String,
    projects: Vec<String>,
    scopes: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    bytes: Bytes,
) -> Result<Response, ApiError> {
    caller.require(auth::ADMIN)?;
    let new: NewKey = body(&bytes)?;
    let mut problems = auth::validate_key(&new.name, &new.projects, &new.scopes);
    if new.expires_at.is_some_and(|at| at <= Utc::now()) {
        problems.push("expires_at must be in the future".to_owned());
    }
    if !problems.is_empty() {
        return Err(ApiError::invalid("invalid key", &problems));
    }
    // A key cannot hand out more projects than it sees itself.
    if !new.projects.iter().all(|project| caller.sees(project)) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "projects beyond the caller's own"));
    }
    let (key, token) =
        auth::create_key(&state.db, &new.name, &new.projects, &new.scopes, new.expires_at)
            .await
            .map_err(|err| {
                if auth::is_unique_violation(&err) {
                    ApiError::new(StatusCode::CONFLICT, "a live key already has this name")
                } else {
                    err.into()
                }
            })?;
    tracing::info!(actor = %caller.actor, key = %key.name, prefix = %key.prefix, "key created");
    let mut created = serde_json::to_value(&key).expect("a key serialises");
    created["token"] = json!(token);
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

pub async fn revoke(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    caller.require(auth::ADMIN)?;
    let revoked =
        sqlx::query("UPDATE api_keys SET revoked_at = COALESCE(revoked_at, now()) WHERE id = $1")
            .bind(id(&key)?)
            .execute(&state.db)
            .await?;
    if revoked.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    tracing::info!(actor = %caller.actor, %key, "key revoked");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
struct Access {
    #[serde(skip)]
    id: i64,
    at: DateTime<Utc>,
    key_id: Uuid,
    key_name: String,
    method: String,
    path: String,
    status: i32,
    remote_addr: Option<String>,
}

pub async fn access(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<Caller>,
    RawQuery(query): RawQuery,
) -> Result<Json<Value>, ApiError> {
    caller.require(auth::ADMIN)?;
    let params = params(query.as_deref());
    let limit = limit(&params)?;
    let key = param(&params, "key")
        .map(|text| Uuid::parse_str(text).map_err(|_| ApiError::bad_request("invalid key")))
        .transpose()?;
    let before: Option<i64> = cursor(&params)?;
    let rows: Vec<Access> = sqlx::query_as(
        "SELECT l.id, l.at, l.key_id, k.name AS key_name, l.method, l.path, l.status, \
         l.remote_addr FROM access_log l JOIN api_keys k ON k.id = l.key_id \
         WHERE ($1::uuid IS NULL OR l.key_id = $1) AND ($2::bigint IS NULL OR l.id < $2) \
         ORDER BY l.id DESC LIMIT $3",
    )
    .bind(key)
    .bind(before)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;
    let (entries, next_cursor) = page(rows, limit, |row| row.id);
    Ok(Json(json!({ "entries": entries, "next_cursor": next_cursor })))
}

//! The `/v1/admin` API. Every route runs behind [`auth::middleware`].

mod hooks;
mod keys;
mod reports;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::auth::{self, Caller, Gate};
use crate::error::ApiError;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

pub fn router(state: Arc<AppState>, trust_client_cert: bool) -> Router {
    let gate = Gate { state: state.clone(), trust_client_cert };
    Router::new()
        .route("/v1/admin/me", get(me))
        .route("/v1/admin/projects", get(reports::projects))
        .route("/v1/admin/reports", get(reports::list))
        .route("/v1/admin/reports/{project}/{id}", get(reports::show).patch(reports::update))
        .route(
            "/v1/admin/reports/{project}/{id}/attachments/{attachment}",
            get(reports::attachment),
        )
        .route(
            "/v1/admin/reports/{project}/{id}/claim",
            post(reports::claim).delete(reports::release),
        )
        .route("/v1/admin/reports/{project}/{id}/events", post(reports::note))
        .route("/v1/admin/keys", get(keys::list).post(keys::create))
        .route("/v1/admin/keys/{id}", axum::routing::delete(keys::revoke))
        .route("/v1/admin/access", get(keys::access))
        .route("/v1/admin/webhooks", get(hooks::list).post(hooks::create))
        .route("/v1/admin/webhooks/{id}", axum::routing::patch(hooks::update).delete(hooks::remove))
        .route("/v1/admin/webhooks/{id}/ping", post(hooks::ping))
        .route("/v1/admin/deliveries", get(hooks::deliveries))
        .route("/v1/admin/deliveries/{id}/redeliver", post(hooks::redeliver))
        .route_layer(from_fn_with_state(gate, auth::middleware))
        .with_state(state)
}

async fn me(Extension(caller): Extension<Caller>) -> axum::Json<Value> {
    axum::Json(json!({
        "actor": caller.actor,
        "scopes": caller.scopes,
        "projects": caller.projects,
    }))
}

fn body<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(bytes)
        .map_err(|err| ApiError::invalid("invalid body", &[err.to_string()]))
}

fn optional_body<T: DeserializeOwned + Default>(bytes: &Bytes) -> Result<T, ApiError> {
    if bytes.iter().all(u8::is_ascii_whitespace) { Ok(T::default()) } else { body(bytes) }
}

fn id(text: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(text).map_err(|_| ApiError::not_found())
}

fn params(query: Option<&str>) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
}

fn limit(params: &[(String, String)]) -> Result<i64, ApiError> {
    match param(params, "limit") {
        None => Ok(DEFAULT_LIMIT),
        Some(text) => text
            .parse::<i64>()
            .ok()
            .filter(|limit| *limit >= 1)
            .map(|limit| limit.min(MAX_LIMIT))
            .ok_or_else(|| ApiError::bad_request("limit must be a positive number")),
    }
}

fn encode_cursor<T: serde::Serialize>(position: &T) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(position).expect("a cursor serialises"))
}

fn cursor<T: DeserializeOwned>(params: &[(String, String)]) -> Result<Option<T>, ApiError> {
    param(params, "cursor")
        .map(|text| {
            URL_SAFE_NO_PAD
                .decode(text)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .ok_or_else(|| ApiError::bad_request("invalid cursor"))
        })
        .transpose()
}

fn page<R, T: serde::Serialize>(
    mut rows: Vec<R>,
    limit: i64,
    position: impl Fn(&R) -> T,
) -> (Vec<R>, Option<String>) {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if rows.len() > limit {
        rows.truncate(limit);
        let next = rows.last().map(|row| encode_cursor(&position(row)));
        (rows, next)
    } else {
        (rows, None)
    }
}

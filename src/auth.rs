//! Who is calling the admin API: a bearer key, or on the admin listener the client certificate
//! subject the gateway forwards.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;

pub const READ: &str = "reports:read";
pub const ATTACHMENTS: &str = "attachments:read";
pub const TRIAGE: &str = "reports:triage";
pub const ADMIN: &str = "admin";
pub const SCOPES: [&str; 4] = [READ, ATTACHMENTS, TRIAGE, ADMIN];

pub const XFCC_HEADER: &str = "x-forwarded-client-cert";
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const ALL_PROJECTS: &str = "*";

#[derive(Debug, Clone)]
pub struct Caller {
    pub actor: String,
    pub key_id: Option<Uuid>,
    pub scopes: Vec<String>,
    pub projects: Vec<String>,
}

impl Caller {
    fn device(common_name: &str) -> Self {
        Self {
            actor: format!("admin:{common_name}"),
            key_id: None,
            scopes: SCOPES.iter().map(|&s| s.to_owned()).collect(),
            projects: vec![ALL_PROJECTS.to_owned()],
        }
    }

    #[must_use]
    pub fn has(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }

    pub fn require(&self, scope: &str) -> Result<(), ApiError> {
        if self.has(scope) { Ok(()) } else { Err(ApiError::forbidden()) }
    }

    #[must_use]
    pub fn sees(&self, project: &str) -> bool {
        self.projects.iter().any(|p| p == ALL_PROJECTS || p == project)
    }

    /// The projects to restrict a query to; `None` when the caller sees all of them.
    #[must_use]
    pub fn visible(&self) -> Option<&[String]> {
        if self.projects.iter().any(|p| p == ALL_PROJECTS) { None } else { Some(&self.projects) }
    }
}

#[must_use]
pub fn random_alnum(len: usize) -> String {
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        getrandom::fill(&mut buf).expect("the system random source is available");
        // 248 is the largest multiple of 62 below 256; larger bytes would bias the draw.
        for byte in buf.iter().filter(|&&b| b < 248) {
            if out.len() < len {
                out.push(char::from(ALPHABET[usize::from(byte % 62)]));
            }
        }
    }
    out
}

#[must_use]
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn split_outside_quotes(input: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch == separator && !quoted {
            parts.push(&input[start..index]);
            start = index + ch.len_utf8();
        }
    }
    parts.push(&input[start..]);
    parts
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value)
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        out.push(if ch == '\\' { chars.next().unwrap_or(ch) } else { ch });
    }
    out
}

fn field<'a>(parts: &[&'a str], name: &str) -> Option<&'a str> {
    parts.iter().find_map(|part| {
        let (key, value) = part.split_once('=')?;
        key.trim().eq_ignore_ascii_case(name).then(|| unquote(value))
    })
}

/// The certificate common name in an `x-forwarded-client-cert` value. With several elements the
/// last one is used: it is the one added by the nearest proxy.
#[must_use]
pub fn xfcc_common_name(header: &str) -> Option<String> {
    let element = split_outside_quotes(header, ',').pop()?;
    // The gateway escapes only the quotes of the subject; its other escapes belong to the name.
    let subject = field(&split_outside_quotes(element, ';'), "subject")?.replace("\\\"", "\"");
    let common_name = unescape(field(&split_outside_quotes(&subject, ','), "cn")?);
    let common_name = common_name.trim();
    (!common_name.is_empty() && common_name.len() <= 128).then(|| common_name.to_owned())
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Key {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub projects: Vec<String>,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[must_use]
pub fn is_project_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[must_use]
pub fn validate_key(name: &str, projects: &[String], scopes: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    if name.trim().is_empty() || name.len() > 100 || name != name.trim() {
        problems.push("name must be 1 to 100 characters".to_owned());
    }
    if projects.is_empty() {
        problems.push("projects must not be empty".to_owned());
    }
    for project in projects {
        if project != ALL_PROJECTS && !is_project_name(project) {
            problems.push(format!("projects: {project:?} is not a project name or *"));
        }
    }
    if scopes.is_empty() {
        problems.push("scopes must not be empty".to_owned());
    }
    for scope in scopes {
        if !SCOPES.contains(&scope.as_str()) {
            problems.push(format!("scopes: {scope:?} is not one of {}", SCOPES.join(", ")));
        }
    }
    problems
}

pub async fn create_key(
    db: &PgPool,
    name: &str,
    projects: &[String],
    scopes: &[String],
    expires_at: Option<DateTime<Utc>>,
) -> sqlx::Result<(Key, String)> {
    let prefix = random_alnum(8);
    let token = format!("hmm_{prefix}_{}", random_alnum(32));
    let key = sqlx::query_as(
        "INSERT INTO api_keys (name, prefix, hash, projects, scopes, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, name, prefix, projects, scopes, created_at, expires_at, revoked_at, last_used_at",
    )
    .bind(name)
    .bind(&prefix)
    .bind(hash_token(&token))
    .bind(projects)
    .bind(scopes)
    .bind(expires_at)
    .fetch_one(db)
    .await?;
    Ok((key, token))
}

#[must_use]
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

#[derive(sqlx::FromRow)]
struct LiveKey {
    id: Uuid,
    name: String,
    projects: Vec<String>,
    scopes: Vec<String>,
    last_used_at: Option<DateTime<Utc>>,
}

async fn key_caller(db: &PgPool, token: &str) -> Result<Caller, ApiError> {
    if !token.starts_with("hmm_") {
        return Err(ApiError::unauthorized());
    }
    let key: Option<LiveKey> = sqlx::query_as(
        "SELECT id, name, projects, scopes, last_used_at FROM api_keys \
         WHERE hash = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(hash_token(token))
    .fetch_optional(db)
    .await?;
    let key = key.ok_or_else(ApiError::unauthorized)?;
    if key.last_used_at.is_none_or(|at| Utc::now() - at > Duration::minutes(1)) {
        sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
            .bind(key.id)
            .execute(db)
            .await?;
    }
    Ok(Caller {
        actor: format!("key:{}", key.name),
        key_id: Some(key.id),
        scopes: key.scopes,
        projects: key.projects,
    })
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

#[derive(Clone)]
pub struct Gate {
    pub state: Arc<AppState>,
    /// Only the admin listener, reachable solely through the gateway, may believe the header.
    pub trust_client_cert: bool,
}

async fn identify(gate: &Gate, headers: &HeaderMap) -> Result<Caller, ApiError> {
    if headers.contains_key(AUTHORIZATION) {
        let token = bearer(headers).ok_or_else(ApiError::unauthorized)?;
        return key_caller(&gate.state.db, token).await;
    }
    if gate.trust_client_cert {
        let common_name = headers
            .get_all(XFCC_HEADER)
            .iter()
            .next_back()
            .and_then(|value| value.to_str().ok())
            .and_then(xfcc_common_name);
        if let Some(common_name) = common_name {
            return Ok(Caller::device(&common_name));
        }
    }
    Err(ApiError::unauthorized())
}

pub async fn middleware(State(gate): State<Gate>, mut request: Request, next: Next) -> Response {
    let caller = match identify(&gate, request.headers()).await {
        Ok(caller) => caller,
        Err(err) => return err.into_response(),
    };
    let key_id = caller.key_id;
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let address = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|list| list.rsplit(',').next())
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    request.extensions_mut().insert(caller);
    let response = next.run(request).await;
    if let Some(key_id) = key_id {
        let logged = sqlx::query(
            "INSERT INTO access_log (key_id, method, path, status, remote_addr) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(key_id)
        .bind(method)
        .bind(path)
        .bind(i32::from(response.status().as_u16()))
        .bind(address)
        .execute(&gate.state.db)
        .await;
        if let Err(err) = logged {
            tracing::error!(error = ?err, "writing the access log");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_common_name_is_read_from_the_subject() {
        assert_eq!(
            xfcc_common_name(r#"Hash=abc123;Subject="CN=macbook-dorsk,O=dorsk""#).as_deref(),
            Some("macbook-dorsk")
        );
        assert_eq!(
            xfcc_common_name(r#"By=spiffe://x;Hash=ab;subject="O=dorsk, cn=phone";URI=spiffe://y"#)
                .as_deref(),
            Some("phone")
        );
        assert_eq!(xfcc_common_name("Subject=CN=bare").as_deref(), Some("bare"));
    }

    #[test]
    fn the_last_element_wins() {
        let header = r#"Hash=1;Subject="CN=forged,O=evil",Hash=2;Subject="CN=real,O=dorsk""#;
        assert_eq!(xfcc_common_name(header).as_deref(), Some("real"));
        let no_subject_last = r#"Hash=1;Subject="CN=forged",Hash=2"#;
        assert_eq!(xfcc_common_name(no_subject_last), None);
    }

    #[test]
    fn separators_inside_quotes_and_escapes_are_kept() {
        let header = r#"Hash=1;Subject="CN=a\,b;c,O=\"quoted\" org""#;
        assert_eq!(xfcc_common_name(header).as_deref(), Some("a,b;c"));
    }

    #[test]
    fn a_value_without_a_common_name_is_refused() {
        for header in ["", "Hash=abc", r#"Subject="""#, r#"Subject="O=dorsk""#, r#"Subject="CN=""#]
        {
            assert_eq!(xfcc_common_name(header), None, "{header}");
        }
    }

    #[test]
    fn tokens_are_alphanumeric_and_hashed() {
        let secret = random_alnum(32);
        assert_eq!(secret.len(), 32);
        assert!(secret.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(secret, random_alnum(32));
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

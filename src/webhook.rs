//! Webhook outbox and delivery. A delivery row is written in the transaction of the change it
//! reports; the worker claims pending rows, posts them and records the outcome.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::PgConnection;
use tokio::sync::watch;
use tokio::task::JoinSet;
use url::{Host, Url};
use uuid::Uuid;

use crate::{AppState, events};

pub const CREATED: &str = "report.created";
pub const STATUS_CHANGED: &str = "report.status_changed";
pub const PING: &str = "ping";
pub const SUBSCRIBABLE: [&str; 2] = [CREATED, STATUS_CHANGED];

const TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_secs(5);
const BATCH: i64 = 10;
/// How long a claimed delivery is hidden from other workers; longer than any attempt can take.
const LEASE_SECONDS: f64 = 120.0;

pub struct Settings {
    /// Hosts that may use http and resolve to private addresses.
    pub allow_hosts: Vec<String>,
    /// Wait before each retry; a delivery fails once they are used up.
    pub backoff: Vec<Duration>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            allow_hosts: Vec::new(),
            backoff: [60, 300, 1800, 7200, 43200].map(Duration::from_secs).to_vec(),
        }
    }
}

impl Settings {
    #[must_use]
    pub fn allowing(hosts: &[String]) -> Self {
        Self {
            allow_hosts: hosts
                .iter()
                .map(|h| h.trim().to_ascii_lowercase())
                .filter(|h| !h.is_empty())
                .collect(),
            ..Self::default()
        }
    }

    fn allows(&self, url: &Url) -> bool {
        let host = url.host_str().unwrap_or_default();
        let host = host.trim_start_matches('[').trim_end_matches(']');
        self.allow_hosts.iter().any(|allowed| allowed.eq_ignore_ascii_case(host))
    }

    pub fn parse_url(&self, input: &str) -> Result<Url, String> {
        let url = Url::parse(input).map_err(|_| "url is not valid".to_owned())?;
        if url.host().is_none() || !url.username().is_empty() || url.password().is_some() {
            return Err("url must have a host and no credentials".to_owned());
        }
        match url.scheme() {
            "https" => Ok(url),
            "http" if self.allows(&url) => Ok(url),
            _ => Err("url must be https".to_owned()),
        }
    }

    pub async fn resolve(&self, url: &Url) -> Result<Vec<SocketAddr>, String> {
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses: Vec<SocketAddr> = match url.host() {
            Some(Host::Ipv4(ip)) => vec![SocketAddr::new(ip.into(), port)],
            Some(Host::Ipv6(ip)) => vec![SocketAddr::new(ip.into(), port)],
            Some(Host::Domain(domain)) => tokio::net::lookup_host((domain, port))
                .await
                .map_err(|_| "url host does not resolve".to_owned())?
                .collect(),
            None => Vec::new(),
        };
        if addresses.is_empty() {
            return Err("url host does not resolve".to_owned());
        }
        if !self.allows(url) && addresses.iter().any(|a| is_internal(a.ip())) {
            return Err("url resolves to a private address".to_owned());
        }
        Ok(addresses)
    }

    pub async fn check_url(&self, input: &str) -> Result<Url, String> {
        let url = self.parse_url(input)?;
        self.resolve(&url).await?;
        Ok(url)
    }
}

fn is_internal_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || (a == 100 && (64..128).contains(&b))
}

#[must_use]
pub fn is_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_internal_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_internal_v4(mapped);
            }
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80
        }
    }
}

#[must_use]
pub fn sign(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[derive(sqlx::FromRow)]
struct ReportFacts {
    kind: String,
    category: Option<String>,
    comment: String,
    page_url: Option<String>,
    status: String,
    received_at: DateTime<Utc>,
}

/// Queues `event` about a report for each enabled webhook of its project that wants it. Runs on
/// the connection of the transaction that made the change.
pub async fn enqueue(
    conn: &mut PgConnection,
    project: &str,
    report_id: &str,
    event: &str,
    extra: Value,
) -> sqlx::Result<u64> {
    let hooks: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM webhooks WHERE project = $1 AND enabled AND $2 = ANY(events)",
    )
    .bind(project)
    .bind(event)
    .fetch_all(&mut *conn)
    .await?;
    if hooks.is_empty() {
        return Ok(0);
    }
    let report: ReportFacts = sqlx::query_as(
        "SELECT kind, category, comment, page_url, status, received_at FROM reports \
         WHERE project = $1 AND id = $2",
    )
    .bind(project)
    .bind(report_id)
    .fetch_one(&mut *conn)
    .await?;
    let mut payload = json!({
        "event": event,
        "at": Utc::now(),
        "project": project,
        "report": {
            "id": report_id,
            "kind": report.kind,
            "category": report.category,
            "comment": report.comment,
            "page_url": report.page_url,
            "status": report.status,
            "received_at": report.received_at,
        },
    });
    if let (Some(payload), Value::Object(extra)) = (payload.as_object_mut(), extra) {
        payload.extend(extra);
    }
    let queued = sqlx::query(
        "INSERT INTO webhook_deliveries (webhook_id, event, project, report_id, payload) \
         SELECT id, $2, $3, $4, $5 FROM unnest($1::uuid[]) AS id",
    )
    .bind(hooks)
    .bind(event)
    .bind(project)
    .bind(report_id)
    .bind(payload)
    .execute(&mut *conn)
    .await?;
    Ok(queued.rows_affected())
}

#[derive(sqlx::FromRow)]
struct Claimed {
    id: Uuid,
    event: String,
    project: String,
    report_id: Option<String>,
    payload: Value,
    attempts: i32,
    name: String,
    url: String,
    secret: String,
    enabled: bool,
}

struct Outcome {
    status_code: Option<i32>,
    error: Option<String>,
    duration_ms: i32,
    retry: bool,
}

impl Outcome {
    fn refused(error: String, retry: bool) -> Self {
        Self { status_code: None, error: Some(error), duration_ms: 0, retry }
    }

    fn delivered(&self) -> bool {
        self.status_code.is_some_and(|code| (200..300).contains(&code))
    }
}

async fn post(settings: &Settings, delivery: &Claimed) -> Outcome {
    if !delivery.enabled && delivery.event != PING {
        return Outcome::refused("webhook is disabled".to_owned(), false);
    }
    let url = match settings.parse_url(&delivery.url) {
        Ok(url) => url,
        Err(error) => return Outcome::refused(error, true),
    };
    let addresses = match settings.resolve(&url).await {
        Ok(addresses) => addresses,
        Err(error) => return Outcome::refused(error, true),
    };
    let mut client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .no_proxy()
        .user_agent(concat!("hmm-server/", env!("CARGO_PKG_VERSION")));
    // Pinned so the connection goes to the addresses that were just checked.
    if let Some(Host::Domain(domain)) = url.host() {
        client = client.resolve_to_addrs(domain, &addresses);
    }
    let client = match client.build() {
        Ok(client) => client,
        Err(err) => return Outcome::refused(err.to_string(), true),
    };

    let mut payload = delivery.payload.clone();
    payload["delivery_id"] = json!(delivery.id);
    let body = payload.to_string().into_bytes();
    let timestamp = Utc::now().timestamp();
    let started = Instant::now();
    let sent = client
        .post(url)
        .header("content-type", "application/json")
        .header("x-hmm-event", &delivery.event)
        .header("x-hmm-delivery", delivery.id.to_string())
        .header("x-hmm-timestamp", timestamp)
        .header("x-hmm-signature", sign(&delivery.secret, timestamp, &body))
        .body(body)
        .send()
        .await;
    let duration_ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
    match sent {
        Ok(response) => Outcome {
            status_code: Some(i32::from(response.status().as_u16())),
            error: None,
            duration_ms,
            retry: true,
        },
        Err(err) => {
            let error = if err.is_timeout() {
                "timed out".to_owned()
            } else if err.is_connect() {
                "connection failed".to_owned()
            } else {
                err.without_url().to_string()
            };
            Outcome { status_code: None, error: Some(error), duration_ms, retry: true }
        }
    }
}

async fn settle(state: &AppState, delivery: &Claimed, outcome: &Outcome) -> sqlx::Result<()> {
    let retry_in = usize::try_from(delivery.attempts - 1)
        .ok()
        .and_then(|index| state.webhooks.backoff.get(index))
        .filter(|_| outcome.retry && !outcome.delivered());
    let next_state = match (outcome.delivered(), retry_in) {
        (true, _) => "delivered",
        (false, Some(_)) => "pending",
        (false, None) => "failed",
    };
    let mut tx = state.db.begin().await?;
    let updated = sqlx::query(
        "UPDATE webhook_deliveries SET state = $2, \
         next_attempt_at = now() + make_interval(secs => $3::double precision), \
         last_status_code = $4, last_error = $5, last_duration_ms = $6, updated_at = now() \
         WHERE id = $1",
    )
    .bind(delivery.id)
    .bind(next_state)
    .bind(retry_in.map(Duration::as_secs_f64))
    .bind(outcome.status_code)
    .bind(&outcome.error)
    .bind(outcome.duration_ms)
    .execute(&mut *tx)
    .await?;
    if let (1, Some(report_id)) = (updated.rows_affected(), &delivery.report_id) {
        events::record(
            &mut *tx,
            &delivery.project,
            report_id,
            events::SYSTEM,
            "webhook",
            json!({
                "webhook": delivery.name,
                "event": delivery.event,
                "attempt": delivery.attempts,
                "status_code": outcome.status_code,
                "error": outcome.error,
            }),
        )
        .await?;
    }
    tx.commit().await
}

pub async fn deliver_due(state: &Arc<AppState>) -> sqlx::Result<usize> {
    let claimed: Vec<Claimed> = sqlx::query_as(
        "WITH due AS ( \
           SELECT id FROM webhook_deliveries \
           WHERE state = 'pending' AND next_attempt_at <= now() \
           ORDER BY next_attempt_at LIMIT $1 FOR UPDATE SKIP LOCKED \
         ) \
         UPDATE webhook_deliveries d \
         SET attempts = d.attempts + 1, updated_at = now(), \
             next_attempt_at = now() + make_interval(secs => $2::double precision) \
         FROM due, webhooks w WHERE d.id = due.id AND w.id = d.webhook_id \
         RETURNING d.id, d.event, d.project, d.report_id, d.payload, d.attempts, \
                   w.name, w.url, w.secret, w.enabled",
    )
    .bind(BATCH)
    .bind(LEASE_SECONDS)
    .fetch_all(&state.db)
    .await?;
    let count = claimed.len();
    let mut attempts = JoinSet::new();
    for delivery in claimed {
        let state = state.clone();
        attempts.spawn(async move {
            let outcome = post(&state.webhooks, &delivery).await;
            if let Err(err) = settle(&state, &delivery, &outcome).await {
                tracing::error!(error = ?err, delivery = %delivery.id, "recording a delivery");
            }
        });
    }
    attempts.join_all().await;
    Ok(count)
}

/// Delivers until `stop` is dropped; an attempt under way is finished first, so its row is not
/// left claimed.
pub async fn run(state: Arc<AppState>, mut stop: watch::Receiver<()>) {
    while stop.has_changed().is_ok() {
        match deliver_due(&state).await {
            Ok(0) => {}
            Ok(_) => continue,
            Err(err) => tracing::error!(error = ?err, "claiming webhook deliveries"),
        }
        tokio::select! {
            _ = stop.changed() => {}
            () = state.wake.notified() => {}
            () = tokio::time::sleep(POLL) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_wait_longer_each_time() {
        let waits: Vec<u64> = Settings::default().backoff.iter().map(Duration::as_secs).collect();
        assert_eq!(waits, [60, 300, 1800, 7200, 43200]);
    }

    #[test]
    fn internal_addresses_are_recognised() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.11.80",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_internal(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "160.251.212.165", "2606:4700:4700::1111"] {
            assert!(!is_internal(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn urls_are_https_and_public_unless_the_host_is_allowed() {
        let strict = Settings::default();
        assert!(strict.check_url("https://1.1.1.1/hook").await.is_ok());
        for refused in [
            "http://1.1.1.1/hook",
            "ftp://1.1.1.1/hook",
            "not a url",
            "https://user:pass@1.1.1.1/",
            "https://127.0.0.1/hook",
            "https://[::1]/hook",
            "https://169.254.169.254/latest/meta-data",
            "https://192.168.1.10:8443/",
        ] {
            assert!(strict.check_url(refused).await.is_err(), "{refused}");
        }
        let lenient = Settings::allowing(&["127.0.0.1".to_owned(), " ::1".to_owned()]);
        assert!(lenient.check_url("http://127.0.0.1:9000/hook").await.is_ok());
        assert!(lenient.check_url("https://[::1]/hook").await.is_ok());
        assert!(lenient.check_url("http://10.0.0.1/hook").await.is_err());
    }

    #[test]
    fn the_signature_is_an_hmac_of_timestamp_and_body() {
        assert_eq!(
            sign("secret", 1_700_000_000, br#"{"a":1}"#),
            "sha256=49f24e537407743fa4a0242bb63b94b9a47ee99cbbe071ccd8a22550ae411686".to_owned(),
        );
    }
}

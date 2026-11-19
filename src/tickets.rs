use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Context;
use regex::Regex;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::sync::watch;
use url::Url;

use crate::{AppState, events};

pub const ACTOR: &str = "sync:youtrack";
const TIMEOUT: Duration = Duration::from_secs(15);

static ISSUE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Z][A-Z0-9_]*-\d+\b").unwrap());

/// The YouTrack instance tickets are checked against.
pub struct TicketSource {
    base: Url,
    token: String,
    client: reqwest::Client,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    pub tickets: usize,
    pub resolved: usize,
    pub reopened: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ticket {
    Resolved,
    Open,
}

impl TicketSource {
    pub fn new(url: &str, token: &str) -> anyhow::Result<Self> {
        let base =
            Url::parse(url.trim_end_matches('/')).context("HMM_YOUTRACK_URL is not a URL")?;
        anyhow::ensure!(base.host_str().is_some(), "HMM_YOUTRACK_URL has no host");
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("hmm-server/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base, token: token.to_owned(), client })
    }

    #[must_use]
    pub fn url(&self) -> &Url {
        &self.base
    }

    /// The issue id a report's ticket names, if it is on this instance.
    #[must_use]
    pub fn issue_id(&self, ticket: &str) -> Option<String> {
        issue_id(ticket, self.base.host_str()?)
    }

    async fn check(&self, id: &str) -> Option<Ticket> {
        match self.fetch(id).await {
            Ok(None) => {
                tracing::debug!(ticket = %id, "not found in YouTrack");
                None
            }
            Ok(ticket) => ticket,
            Err(error) => {
                tracing::warn!(ticket = %id, %error, "checking a ticket in YouTrack");
                None
            }
        }
    }

    async fn fetch(&self, id: &str) -> Result<Option<Ticket>, String> {
        let url = format!(
            "{}/api/issues/{id}?fields=idReadable,resolved",
            self.base.as_str().trim_end_matches('/')
        );
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|err| err.to_string())?;
        match response.status() {
            StatusCode::NOT_FOUND => return Ok(None),
            status if !status.is_success() => return Err(format!("answered {status}")),
            _ => {}
        }
        let body = response.bytes().await.map_err(|err| err.to_string())?;
        let issue: Value = serde_json::from_slice(&body).map_err(|err| err.to_string())?;
        let resolved = issue.get("resolved").is_some_and(|at| !at.is_null());
        Ok(Some(if resolved { Ticket::Resolved } else { Ticket::Open }))
    }
}

#[must_use]
pub fn issue_id(ticket: &str, host: &str) -> Option<String> {
    let ticket = ticket.trim();
    let haystack = match Url::parse(ticket) {
        Ok(url) if url.has_host() => {
            if !url.host_str()?.eq_ignore_ascii_case(host) {
                return None;
            }
            url.path().to_owned()
        }
        _ => ticket.to_owned(),
    };
    ISSUE_ID.find(&haystack).map(|id| id.as_str().to_owned())
}

#[derive(sqlx::FromRow)]
struct Candidate {
    project: String,
    id: String,
    status: String,
    ticket: String,
}

const CANDIDATES: &str = "SELECT r.project, r.id, r.status, r.ticket FROM reports r \
     WHERE r.ticket IS NOT NULL AND (r.status IN ('triaged', 'in_progress') \
     OR (r.status = 'resolved' AND (SELECT e.actor FROM report_events e \
         WHERE e.project = r.project AND e.report_id = r.id AND e.type = 'status' \
         ORDER BY e.id DESC LIMIT 1) = $1))";

pub async fn sync_once(state: &AppState, source: &TicketSource) -> anyhow::Result<SyncStats> {
    let candidates: Vec<Candidate> =
        sqlx::query_as(CANDIDATES).bind(ACTOR).fetch_all(&state.db).await?;
    let mut tally = SyncStats::default();
    let mut tickets: HashMap<String, Option<Ticket>> = HashMap::new();
    for candidate in candidates {
        let Some(issue) = source.issue_id(&candidate.ticket) else { continue };
        let ticket = if let Some(ticket) = tickets.get(&issue) {
            *ticket
        } else {
            tally.tickets += 1;
            let ticket = source.check(&issue).await;
            tickets.insert(issue.clone(), ticket);
            ticket
        };
        let (to, note) = match (ticket, candidate.status.as_str()) {
            (Some(Ticket::Resolved), "triaged" | "in_progress") => {
                ("resolved", format!("{issue} is resolved in YouTrack"))
            }
            (Some(Ticket::Open), "resolved") => {
                ("triaged", format!("{issue} was reopened in YouTrack"))
            }
            _ => continue,
        };
        if apply(state, &candidate, to, &note).await? {
            tracing::info!(
                project = %candidate.project,
                report = %candidate.id,
                ticket = %issue,
                from = %candidate.status,
                to,
                "report status synced from YouTrack"
            );
            if to == "resolved" {
                tally.resolved += 1;
            } else {
                tally.reopened += 1;
            }
        }
    }
    Ok(tally)
}

/// Changes the status unless the report moved since it was read.
async fn apply(
    state: &AppState,
    candidate: &Candidate,
    to: &str,
    note: &str,
) -> sqlx::Result<bool> {
    let mut tx = state.db.begin().await?;
    let current: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT status, ticket FROM reports WHERE project = $1 AND id = $2 FOR UPDATE",
    )
    .bind(&candidate.project)
    .bind(&candidate.id)
    .fetch_optional(&mut *tx)
    .await?;
    if current != Some((candidate.status.clone(), Some(candidate.ticket.clone()))) {
        return Ok(false);
    }
    let (project, id) = (candidate.project.as_str(), candidate.id.as_str());
    let queued = events::change_status(&mut tx, project, id, ACTOR, &candidate.status, to).await?;
    events::record(&mut *tx, project, id, ACTOR, "note", json!({ "note": note })).await?;
    tx.commit().await?;
    if queued > 0 {
        state.wake.notify_one();
    }
    Ok(true)
}

/// Syncs every `every` until `stop` is dropped.
pub async fn run(
    state: Arc<AppState>,
    source: TicketSource,
    every: Duration,
    mut stop: watch::Receiver<()>,
) {
    while stop.has_changed().is_ok() {
        if let Err(err) = sync_once(&state, &source).await {
            tracing::error!(error = ?err, "syncing tickets from YouTrack");
        }
        tokio::select! {
            _ = stop.changed() => {}
            () = tokio::time::sleep(every) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_ids_are_read_from_ids_and_urls_of_this_instance() {
        let host = "youtrack.dorsk.dev";
        let id = |ticket| issue_id(ticket, host);
        assert_eq!(id("KUSA-1363").as_deref(), Some("KUSA-1363"));
        assert_eq!(id(" HMM_2-7 ").as_deref(), Some("HMM_2-7"));
        assert_eq!(
            id("https://youtrack.dorsk.dev/issue/KUSA-1363/Dark-mode-toggle").as_deref(),
            Some("KUSA-1363")
        );
        assert_eq!(id("https://YouTrack.dorsk.dev/issue/KUSA-1").as_deref(), Some("KUSA-1"));
        assert_eq!(id("https://youtrack.example.com/issue/KUSA-1363"), None);
        assert_eq!(id("https://youtrack.dorsk.dev/issues"), None);
        for garbage in ["", "kusa-12", "KUSA-", "-12", "see the board", "12-KUSA"] {
            assert_eq!(id(garbage), None, "{garbage}");
        }
    }
}

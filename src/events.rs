use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgExecutor};

use crate::webhook;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Event {
    pub at: DateTime<Utc>,
    pub actor: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub kind: String,
    pub data: Value,
}

pub const SYSTEM: &str = "system";

pub async fn record<'e>(
    executor: impl PgExecutor<'e>,
    project: &str,
    report_id: &str,
    actor: &str,
    kind: &str,
    data: Value,
) -> sqlx::Result<Event> {
    sqlx::query_as(
        "INSERT INTO report_events (project, report_id, actor, type, data) \
         VALUES ($1, $2, $3, $4, $5) RETURNING at, actor, type, data",
    )
    .bind(project)
    .bind(report_id)
    .bind(actor)
    .bind(kind)
    .bind(data)
    .fetch_one(executor)
    .await
}

pub async fn of_report<'e>(
    executor: impl PgExecutor<'e>,
    project: &str,
    report_id: &str,
) -> sqlx::Result<Vec<Event>> {
    sqlx::query_as(
        "SELECT at, actor, type, data FROM report_events \
         WHERE project = $1 AND report_id = $2 ORDER BY id",
    )
    .bind(project)
    .bind(report_id)
    .fetch_all(executor)
    .await
}

/// Sets a report's status, records the event and queues the webhooks; returns how many were
/// queued.
pub async fn change_status(
    conn: &mut PgConnection,
    project: &str,
    report_id: &str,
    actor: &str,
    from: &str,
    to: &str,
) -> sqlx::Result<u64> {
    sqlx::query("UPDATE reports SET status = $3 WHERE project = $1 AND id = $2")
        .bind(project)
        .bind(report_id)
        .bind(to)
        .execute(&mut *conn)
        .await?;
    let change = json!({ "from": from, "to": to });
    record(&mut *conn, project, report_id, actor, "status", change.clone()).await?;
    webhook::enqueue(conn, project, report_id, webhook::STATUS_CHANGED, change).await
}

//! Ticket sync against a fake YouTrack. Needs a throwaway Postgres behind `DATABASE_URL_TEST` and
//! is skipped without it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use hmm_server::policy::Policy;
use hmm_server::tickets::{self, TicketSource};
use hmm_server::{AppState, MIGRATOR, build_router};
use object_store::ObjectStore;
use object_store::memory::InMemory;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

const TOKEN: &str = "perm-readonly-token";

#[derive(Clone, Default)]
struct YouTrack {
    resolved: Arc<Mutex<HashMap<String, bool>>>,
    auth: Arc<Mutex<Vec<String>>>,
}

impl YouTrack {
    async fn start(issues: &[(&str, bool)]) -> (Self, String) {
        let youtrack = Self::default();
        youtrack.resolved.lock().unwrap().extend(issues.iter().map(|&(id, r)| (id.to_owned(), r)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let router =
            Router::new()
                .route(
                    "/api/issues/{id}",
                    axum::routing::get(
                        |State(youtrack): State<Self>,
                         Path(id): Path<String>,
                         headers: HeaderMap| async move {
                            let auth = headers
                                .get("authorization")
                                .map(|v| v.to_str().unwrap().to_owned());
                            youtrack.auth.lock().unwrap().push(auth.unwrap_or_default());
                            match youtrack.resolved.lock().unwrap().get(&id) {
                                None => StatusCode::NOT_FOUND.into_response(),
                                Some(&resolved) => axum::Json(json!({
                                    "idReadable": id,
                                    "resolved": resolved.then_some(1_763_500_000_000_i64),
                                    "$type": "Issue",
                                }))
                                .into_response(),
                            }
                        },
                    ),
                )
                .with_state(youtrack.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (youtrack, url)
    }

    fn set(&self, id: &str, resolved: bool) {
        self.resolved.lock().unwrap().insert(id.to_owned(), resolved);
    }
}

async fn ingest(public: &Router, project: &str, id: &str) {
    let mut report: Value = serde_json::from_str(include_str!("fixtures/sample.json")).unwrap();
    report["project"] = json!(project);
    report["id"] = json!(id);
    let request = Request::post("/v1/reports")
        .header("content-type", "application/json")
        .header("x-hmm-key", project)
        .body(Body::from(report.to_string()))
        .unwrap();
    assert_eq!(public.clone().oneshot(request).await.unwrap().status(), StatusCode::ACCEPTED);
}

async fn set(db: &PgPool, project: &str, id: &str, status: &str, ticket: &str) {
    sqlx::query("UPDATE reports SET status = $3, ticket = $4 WHERE project = $1 AND id = $2")
        .bind(project)
        .bind(id)
        .bind(status)
        .bind(ticket)
        .execute(db)
        .await
        .unwrap();
}

async fn status(db: &PgPool, project: &str, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM reports WHERE project = $1 AND id = $2")
        .bind(project)
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn sync_events(db: &PgPool, project: &str, id: &str) -> Vec<(String, Value)> {
    sqlx::query_as(
        "SELECT type, data FROM report_events \
         WHERE project = $1 AND report_id = $2 AND actor = $3 ORDER BY id",
    )
    .bind(project)
    .bind(id)
    .bind(tickets::ACTOR)
    .fetch_all(db)
    .await
    .unwrap()
}

async fn app() -> Option<(Arc<AppState>, Router, PgPool)> {
    let Ok(url) = std::env::var("DATABASE_URL_TEST") else {
        eprintln!("DATABASE_URL_TEST is not set, skipping");
        return None;
    };
    let db = PgPoolOptions::new().max_connections(5).connect(&url).await.unwrap();
    MIGRATOR.run(&db).await.unwrap();
    let blobs = Arc::new(InMemory::new()) as Arc<dyn ObjectStore>;
    let app = Arc::new(AppState::new(db.clone(), blobs, Policy::parse(None).unwrap()));
    let public = build_router(app.clone(), 32 * 1024 * 1024);
    Some((app, public, db))
}

async fn webhook(db: &PgPool, project: &str) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO webhooks (name, project, url, events, secret) \
         VALUES ('sync', $1, 'https://hooks.invalid/x', '{report.status_changed}', 's') RETURNING id",
    )
    .bind(project)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn event_count(db: &PgPool, project: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM report_events WHERE project = $1")
        .bind(project)
        .fetch_one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn reports_follow_their_youtrack_tickets() {
    let Some((app, public, db)) = app().await else { return };
    let project = format!("synced-{}", std::process::id());
    let issue = |n: u32| format!("T{}-{n}", std::process::id());
    let (done, open, gone, flip) = (issue(1), issue(2), issue(3), issue(4));
    let (youtrack, base) = YouTrack::start(&[(&done, true), (&open, false), (&flip, true)]).await;
    let source = TicketSource::new(&format!("{base}/"), TOKEN).unwrap();

    let hook = webhook(&db, &project).await;
    let reports = [
        ("triaged-done", "triaged", done.clone()),
        ("progress-done", "in_progress", format!("{base}/issue/{done}/Dark-mode")),
        ("triaged-open", "triaged", open.clone()),
        ("triaged-gone", "triaged", gone.clone()),
        ("triaged-flip", "triaged", flip.clone()),
        ("human-resolved", "resolved", open.clone()),
        ("fresh", "new", done.clone()),
        ("foreign", "triaged", format!("https://youtrack.example.com/issue/{done}")),
        ("garbage", "triaged", "see the board".to_owned()),
    ];
    for (id, status, ticket) in &reports {
        ingest(&public, &project, id).await;
        set(&db, &project, id, status, ticket).await;
    }
    sqlx::query(
        "INSERT INTO report_events (project, report_id, actor, type, data) \
         VALUES ($1, 'human-resolved', 'key:triager', 'status', '{\"from\":\"triaged\",\"to\":\"resolved\"}')",
    )
    .bind(&project)
    .execute(&db)
    .await
    .unwrap();

    let stats = tickets::sync_once(&app, &source).await.unwrap();
    assert_eq!((stats.resolved, stats.reopened), (3, 0), "{stats:?}");
    for id in ["triaged-done", "progress-done", "triaged-flip"] {
        assert_eq!(status(&db, &project, id).await, "resolved", "{id}");
    }
    let from = |id: &str| reports.iter().find(|r| r.0 == id).unwrap().1;
    for (id, n) in [("triaged-done", &done), ("progress-done", &done), ("triaged-flip", &flip)] {
        assert_eq!(
            sync_events(&db, &project, id).await,
            [
                ("status".to_owned(), json!({ "from": from(id), "to": "resolved" })),
                ("note".to_owned(), json!({ "note": format!("{n} is resolved in YouTrack") })),
            ],
            "{id}"
        );
    }
    for (id, expected) in [
        ("triaged-open", "triaged"),
        ("triaged-gone", "triaged"),
        ("human-resolved", "resolved"),
        ("fresh", "new"),
        ("foreign", "triaged"),
        ("garbage", "triaged"),
    ] {
        assert_eq!(status(&db, &project, id).await, expected, "{id}");
        assert!(sync_events(&db, &project, id).await.is_empty(), "{id}");
    }
    let queued: Vec<String> = sqlx::query_scalar(
        "SELECT report_id FROM webhook_deliveries WHERE webhook_id = $1 ORDER BY report_id",
    )
    .bind(hook)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(queued, ["progress-done", "triaged-done", "triaged-flip"]);

    let auth = youtrack.auth.lock().unwrap().clone();
    assert!(auth.len() >= 4, "{auth:?}");
    assert!(auth.iter().all(|header| *header == format!("Bearer {TOKEN}")), "{auth:?}");

    youtrack.set(&flip, false);
    let stats = tickets::sync_once(&app, &source).await.unwrap();
    assert_eq!((stats.resolved, stats.reopened), (0, 1), "{stats:?}");
    assert_eq!(status(&db, &project, "triaged-flip").await, "triaged");
    assert_eq!(
        sync_events(&db, &project, "triaged-flip").await[2..],
        [
            ("status".to_owned(), json!({ "from": "resolved", "to": "triaged" })),
            ("note".to_owned(), json!({ "note": format!("{flip} was reopened in YouTrack") })),
        ]
    );
    assert_eq!(status(&db, &project, "human-resolved").await, "resolved");

    let events = event_count(&db, &project).await;
    let stats = tickets::sync_once(&app, &source).await.unwrap();
    assert_eq!((stats.resolved, stats.reopened), (0, 0), "{stats:?}");
    let after = event_count(&db, &project).await;
    assert_eq!(after, events);

    sqlx::query("DELETE FROM webhooks WHERE id = $1").bind(hook).execute(&db).await.unwrap();
}

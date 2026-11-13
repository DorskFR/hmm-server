//! End-to-end ingest tests. They need a throwaway Postgres behind `DATABASE_URL_TEST` and are
//! skipped without it. Each test uses its own project key, so they share one database.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::TryStreamExt;
use hmm_server::policy::Policy;
use hmm_server::{AppState, MIGRATOR, build_router};
use http_body_util::BodyExt;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

struct App {
    router: Router,
    db: PgPool,
    blobs: Arc<InMemory>,
}

async fn app(policy: Option<&str>) -> Option<App> {
    let Ok(url) = std::env::var("DATABASE_URL_TEST") else {
        eprintln!("DATABASE_URL_TEST is not set, skipping");
        return None;
    };
    let db = PgPoolOptions::new().max_connections(2).connect(&url).await.unwrap();
    MIGRATOR.run(&db).await.unwrap();
    let blobs = Arc::new(InMemory::new());
    let state = Arc::new(AppState::new(
        db.clone(),
        blobs.clone() as Arc<dyn ObjectStore>,
        Policy::parse(policy).unwrap(),
    ));
    Some(App { router: build_router(state, 32 * 1024 * 1024), db, blobs })
}

fn report(project: &str) -> Value {
    let mut report: Value = serde_json::from_str(include_str!("fixtures/sample.json")).unwrap();
    report["project"] = json!(project);
    report["id"] = json!(format!("{project}-{}", std::process::id()));
    report
}

async fn post(
    app: &App,
    key: Option<&str>,
    origin: Option<&str>,
    body: String,
) -> (StatusCode, Value) {
    let mut request = Request::post("/v1/reports").header("content-type", "application/json");
    if let Some(key) = key {
        request = request.header("x-hmm-key", key);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    let response =
        app.router.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn send(app: &App, report: &Value) -> (StatusCode, Value) {
    post(app, report["project"].as_str(), None, report.to_string()).await
}

async fn stored(app: &App, report: &Value) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM reports WHERE project = $1 AND id = $2")
        .bind(report["project"].as_str())
        .bind(report["id"].as_str())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_report_is_stored_with_its_images_in_the_object_store() {
    let Some(app) = app(None).await else { return };
    let mut report = report("stores");
    report["comment"] = json!("login fails with token=abcd1234secret");
    let id = report["id"].as_str().unwrap().to_owned();

    let (status, body) = send(&app, &report).await;
    assert_eq!((status, &body["id"]), (StatusCode::ACCEPTED, &json!(id)));

    let (comment, kind, envelope): (String, String, sqlx::types::Json<Value>) = sqlx::query_as(
        "SELECT comment, kind, envelope FROM reports WHERE project = 'stores' AND id = $1",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(comment, "login fails with token=[redacted]");
    assert_eq!(kind, "feedback");
    assert_eq!(envelope["attachments"][0].get("data"), None);
    assert_eq!(envelope["attachments"][0]["key"], json!(format!("stores/{id}/att-1.png")));
    assert_eq!(envelope["items"][0]["target"]["selector"], "#buy");

    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT id, bytes, object_key FROM attachments WHERE project = 'stores' AND report_id = $1 ORDER BY id",
    )
    .bind(&id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    let objects: Vec<_> = app.blobs.list(None).try_collect().await.unwrap();
    assert_eq!(objects.len(), 2);
    let png = app.blobs.get(&Path::from(rows[0].2.as_str())).await.unwrap().bytes().await.unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    assert_eq!(i64::try_from(png.len()).unwrap(), rows[0].1);

    let events: Vec<(String, String, sqlx::types::Json<Value>)> = sqlx::query_as(
        "SELECT actor, type, data FROM report_events WHERE project = 'stores' AND report_id = $1",
    )
    .bind(&id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].0.as_str(), events[0].1.as_str()), ("system", "received"));
    assert_eq!(events[0].2.0, json!({ "attachments": 2, "bytes": rows[0].1 + rows[1].1 }));
}

#[tokio::test]
async fn a_repeated_report_is_acknowledged_and_stored_once() {
    let Some(app) = app(None).await else { return };
    let report = report("repeats");
    assert_eq!(send(&app, &report).await.0, StatusCode::ACCEPTED);
    let (status, body) = send(&app, &report).await;
    assert_eq!((status, &body["duplicate"]), (StatusCode::OK, &json!(true)));
    assert_eq!(stored(&app, &report).await, 1);
}

#[tokio::test]
async fn bad_requests_are_refused_and_nothing_is_stored() {
    let Some(app) = app(None).await else { return };
    let report = report("refuses");

    let (status, _) = post(&app, None, None, report.to_string()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = post(&app, Some("another"), None, report.to_string()).await;
    assert_eq!((status, &body["error"]), (StatusCode::BAD_REQUEST, &json!("project key mismatch")));
    let (status, _) = post(&app, Some("refuses"), None, "{not json".into()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut invalid = report.clone();
    invalid["kind"] = json!("spam");
    let (status, body) = send(&app, &invalid).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["details"][0].as_str().unwrap().contains("envelope/kind"), "{body}");

    let mut not_png = report.clone();
    not_png["attachments"][0]["data"] = json!("aGVsbG8gd29ybGQ=");
    let (status, body) = send(&app, &not_png).await;
    assert_eq!(
        (status, &body["error"]),
        (StatusCode::BAD_REQUEST, &json!("an attachment is not a PNG image"))
    );

    assert_eq!(stored(&app, &report).await, 0);
    assert_eq!(app.blobs.list(None).try_collect::<Vec<_>>().await.unwrap().len(), 0);
}

#[tokio::test]
async fn a_browser_preflight_is_answered() {
    let Some(app) = app(None).await else { return };
    let request = Request::options("/v1/reports")
        .header("origin", "https://shop.example")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type,x-hmm-key")
        .body(Body::empty())
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    let allowed =
        response.headers()["access-control-allow-headers"].to_str().unwrap().to_lowercase();
    assert!(allowed.contains("x-hmm-key") && allowed.contains("content-type"), "{allowed}");
}

#[tokio::test]
async fn a_policy_limits_projects_origins_rate_and_storage() {
    let policy = r#"{"projects":{"gated":{"origins":["https://shop.example"]},"paced":{}},"rate_per_minute":1}"#;
    let Some(app) = app(Some(policy)).await else { return };

    let unknown = report("ungated");
    assert_eq!(
        send(&app, &unknown).await,
        (StatusCode::FORBIDDEN, json!({"error": "unknown project"}))
    );

    let gated = report("gated");
    let (status, body) =
        post(&app, Some("gated"), Some("https://evil.example"), gated.to_string()).await;
    assert_eq!((status, &body["error"]), (StatusCode::FORBIDDEN, &json!("origin not allowed")));
    let (status, _) =
        post(&app, Some("gated"), Some("https://shop.example"), gated.to_string()).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let mut paced = report("paced");
    assert_eq!(send(&app, &paced).await.0, StatusCode::ACCEPTED);
    paced["id"] = json!("paced-second");
    assert_eq!(send(&app, &paced).await.0, StatusCode::TOO_MANY_REQUESTS);

    let Some(full) = self::app(Some(r#"{"storage_budget_bytes":0}"#)).await else { return };
    let (status, body) = send(&full, &report("budgeted")).await;
    assert_eq!(
        (status, &body["error"]),
        (StatusCode::INSUFFICIENT_STORAGE, &json!("storage budget reached"))
    );
}

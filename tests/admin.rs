//! End-to-end admin API and webhook tests. They need a throwaway Postgres behind
//! `DATABASE_URL_TEST` and are skipped without it. Each test uses its own projects, suffixed with
//! the process id so a rerun against the same database starts clean.
//!
//! Every app shares the webhook settings below: any test's worker may deliver another test's
//! pending rows, so they must all agree on what is allowed and how retries are spaced.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use hmm_server::policy::Policy;
use hmm_server::{AppState, MIGRATOR, auth, build_admin_router, build_router, webhook};
use http_body_util::BodyExt;
use object_store::ObjectStore;
use object_store::memory::InMemory;
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

const XFCC: &str = r#"Hash=abc;Subject="CN=macbook-dorsk,O=dorsk""#;
const READ: &[&str] = &["reports:read"];
const AGENT: &[&str] = &["reports:read", "attachments:read", "reports:triage"];

struct App {
    public: Router,
    admin: Router,
    state: Arc<AppState>,
    db: PgPool,
}

async fn app_with(allow_local_webhooks: bool, ui_dir: Option<&std::path::Path>) -> Option<App> {
    let Ok(url) = std::env::var("DATABASE_URL_TEST") else {
        eprintln!("DATABASE_URL_TEST is not set, skipping");
        return None;
    };
    let db = PgPoolOptions::new().max_connections(5).connect(&url).await.unwrap();
    MIGRATOR.run(&db).await.unwrap();
    let blobs = Arc::new(InMemory::new()) as Arc<dyn ObjectStore>;
    let mut state = AppState::new(db.clone(), blobs, Policy::parse(None).unwrap());
    if allow_local_webhooks {
        state.webhooks = webhook::Settings {
            allow_hosts: vec!["127.0.0.1".to_owned()],
            backoff: vec![Duration::ZERO; 2],
        };
    }
    if let Some(dir) = ui_dir {
        dir.clone_into(&mut state.ui_dir);
    }
    let state = Arc::new(state);
    Some(App {
        public: build_router(state.clone(), 32 * 1024 * 1024),
        admin: build_admin_router(state.clone()),
        state,
        db,
    })
}

async fn app() -> Option<App> {
    app_with(true, None).await
}

fn unique(name: &str) -> String {
    format!("{name}-{}", std::process::id())
}

async fn key(app: &App, name: &str, projects: &[&str], scopes: &[&str]) -> String {
    let owned = |items: &[&str]| items.iter().map(|&s| s.to_owned()).collect::<Vec<_>>();
    auth::create_key(&app.db, &unique(name), &owned(projects), &owned(scopes), None)
        .await
        .unwrap()
        .1
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Bytes,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
}

async fn send(router: &Router, request: Request<Body>) -> Reply {
    let response = router.clone().oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    Reply { status, headers, bytes: response.into_body().collect().await.unwrap().to_bytes() }
}

fn request(method: Method, path: &str, token: Option<&str>, body: Option<&Value>) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    request.body(body.map_or_else(Body::empty, |body| Body::from(body.to_string()))).unwrap()
}

async fn call(
    app: &App,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let reply = send(&app.public, request(method, path, Some(token), body.as_ref())).await;
    (reply.status, reply.json())
}

async fn device(app: &App, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = request(method, path, None, body.as_ref());
    request.headers_mut().insert(auth::XFCC_HEADER, XFCC.parse().unwrap());
    let reply = send(&app.admin, request).await;
    (reply.status, reply.json())
}

async fn ingest(app: &App, project: &str, id: &str) -> Value {
    let mut report: Value = serde_json::from_str(include_str!("fixtures/sample.json")).unwrap();
    report["project"] = json!(project);
    report["id"] = json!(id);
    let request = Request::post("/v1/reports")
        .header("content-type", "application/json")
        .header("x-hmm-key", project)
        .header("x-forwarded-for", "203.0.113.4")
        .body(Body::from(report.to_string()))
        .unwrap();
    assert_eq!(send(&app.public, request).await.status, StatusCode::ACCEPTED);
    report
}

fn event_types(report: &Value) -> Vec<&str> {
    report["events"].as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect()
}

fn ids(list: &Value) -> Vec<&str> {
    list["reports"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn a_key_is_held_to_its_scopes_and_projects() {
    let Some(app) = app().await else { return };
    let (mine, theirs) = (unique("scoped"), unique("scoped-other"));
    ingest(&app, &mine, "r1").await;
    ingest(&app, &theirs, "r1").await;
    let reader = key(&app, "scoped-reader", &[&mine], READ).await;
    let report = format!("/v1/admin/reports/{mine}/r1");

    let anonymous = send(&app.public, request(Method::GET, &report, None, None)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.json(), json!({ "error": "unauthorized" }));
    for token in ["hmm_aaaaaaaa_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "nonsense"] {
        assert_eq!(call(&app, Method::GET, &report, token, None).await.0, StatusCode::UNAUTHORIZED);
    }

    let (status, shown) = call(&app, Method::GET, &report, &reader, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shown["origin"], Value::Null);
    assert_eq!(shown.get("remote_addr"), None);
    assert_eq!(shown["attachments"].as_array().unwrap().len(), 2);
    assert_eq!(shown["attachments"][0]["mime"], "image/png");
    assert_eq!(shown["envelope"]["id"], "r1");
    assert_eq!(event_types(&shown), ["received"]);
    assert_eq!(shown["events"][0]["actor"], "system");
    assert_eq!(shown["events"][0]["data"]["attachments"], 2);

    let (status, me) = call(&app, Method::GET, "/v1/admin/me", &reader, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["actor"], json!(format!("key:{}", unique("scoped-reader"))));
    assert_eq!((&me["scopes"], &me["projects"]), (&json!(READ), &json!([mine])));

    let forbidden = [
        (Method::PATCH, report.clone(), Some(json!({ "status": "triaged" }))),
        (Method::POST, format!("{report}/claim"), None),
        (Method::DELETE, format!("{report}/claim"), None),
        (Method::POST, format!("{report}/events"), Some(json!({ "note": "hi" }))),
        (Method::GET, format!("{report}/attachments/att-1"), None),
        (Method::GET, "/v1/admin/keys".to_owned(), None),
        (Method::GET, "/v1/admin/access".to_owned(), None),
        (Method::GET, "/v1/admin/webhooks".to_owned(), None),
        (Method::GET, "/v1/admin/deliveries".to_owned(), None),
    ];
    for (method, path, body) in forbidden {
        let (status, _) = call(&app, method.clone(), &path, &reader, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
    }

    let other = format!("/v1/admin/reports/{theirs}/r1");
    assert_eq!(call(&app, Method::GET, &other, &reader, None).await.0, StatusCode::NOT_FOUND);
    let missing = format!("/v1/admin/reports/{mine}/nope");
    assert_eq!(call(&app, Method::GET, &missing, &reader, None).await.0, StatusCode::NOT_FOUND);
    let (_, all) = call(&app, Method::GET, "/v1/admin/reports?limit=200", &reader, None).await;
    assert_eq!(ids(&all), ["r1"]);
    assert_eq!(all["reports"][0]["project"], json!(mine));
    let elsewhere = format!("/v1/admin/reports?project={theirs}");
    assert_eq!(ids(&call(&app, Method::GET, &elsewhere, &reader, None).await.1), [""; 0]);

    let (_, projects) = call(&app, Method::GET, "/v1/admin/projects", &reader, None).await;
    assert_eq!(
        projects,
        json!({ "projects": [{ "project": mine, "counts":
            { "new": 1, "triaged": 0, "in_progress": 0, "resolved": 0, "dismissed": 0 } }] })
    );

    let admin = key(&app, "scoped-admin", &["*"], &["reports:read", "admin"]).await;
    let (_, shown) = call(&app, Method::GET, &other, &admin, None).await;
    assert_eq!(shown.get("remote_addr"), None, "the sender address is never stored");
}

#[tokio::test]
async fn keys_are_created_once_hashed_and_die_when_revoked_or_expired() {
    let Some(app) = app().await else { return };
    let name = unique("lifecycle");
    let new = json!({ "name": name, "projects": ["*"], "scopes": ["reports:read", "admin"] });
    let (status, created) = device(&app, Method::POST, "/v1/admin/keys", Some(new.clone())).await;
    assert_eq!(status, StatusCode::CREATED);
    let token = created["token"].as_str().unwrap().to_owned();
    let parts: Vec<&str> = token.split('_').collect();
    assert_eq!((parts[0], parts[1].len(), parts[2].len()), ("hmm", 8, 32));
    assert_eq!(created["prefix"], parts[1]);
    assert!(token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));

    let (hash, stored): (String, String) =
        sqlx::query_as("SELECT hash, to_jsonb(k)::text FROM api_keys k WHERE name = $1")
            .bind(&name)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(hash, auth::hash_token(&token));
    assert!(!stored.contains(parts[2]), "the secret is not stored");

    assert_eq!(
        device(&app, Method::POST, "/v1/admin/keys", Some(new)).await.0,
        StatusCode::CONFLICT
    );
    let invalid = json!({ "name": "", "projects": [], "scopes": ["root"] });
    let (status, body) = device(&app, Method::POST, "/v1/admin/keys", Some(invalid)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["details"].as_array().unwrap().len(), 3, "{body}");

    let (status, listed) = call(&app, Method::GET, "/v1/admin/keys", &token, None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = listed["keys"].as_array().unwrap().iter().find(|k| k["name"] == json!(name));
    let listed = listed.unwrap();
    assert_eq!(listed.get("token"), None);
    assert_eq!(listed.get("hash"), None);
    assert_eq!(listed["revoked_at"], Value::Null);
    assert!(listed["last_used_at"].is_string(), "{listed}");

    let narrow = key(&app, "lifecycle-narrow", &["one"], &["admin"]).await;
    let wider =
        json!({ "name": unique("lifecycle-wider"), "projects": ["*"], "scopes": ["admin"] });
    assert_eq!(
        call(&app, Method::POST, "/v1/admin/keys", &narrow, Some(wider)).await.0,
        StatusCode::FORBIDDEN
    );

    let path = format!("/v1/admin/keys/{}", created["id"].as_str().unwrap());
    assert_eq!(device(&app, Method::DELETE, &path, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(
        device(&app, Method::DELETE, "/v1/admin/keys/not-a-uuid", None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, Method::GET, "/v1/admin/me", &token, None).await.0,
        StatusCode::UNAUTHORIZED
    );

    let expiring = key(&app, "lifecycle-expiring", &["*"], READ).await;
    assert_eq!(call(&app, Method::GET, "/v1/admin/me", &expiring, None).await.0, StatusCode::OK);
    sqlx::query("UPDATE api_keys SET expires_at = now() - interval '1 second' WHERE name = $1")
        .bind(unique("lifecycle-expiring"))
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        call(&app, Method::GET, "/v1/admin/me", &expiring, None).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn last_use_is_stamped_at_most_once_a_minute() {
    let Some(app) = app().await else { return };
    let name = unique("stamped");
    let token = key(&app, "stamped", &["*"], READ).await;
    let used = || async {
        call(&app, Method::GET, "/v1/admin/me", &token, None).await;
        sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
            "SELECT last_used_at FROM api_keys WHERE name = $1",
        )
        .bind(&name)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    let first = used().await;
    assert_eq!(used().await, first);
    sqlx::query("UPDATE api_keys SET last_used_at = now() - interval '2 minutes' WHERE name = $1")
        .bind(&name)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(used().await >= first);
}

#[tokio::test]
async fn the_list_filters_pages_and_orders() {
    let Some(app) = app().await else { return };
    let project = unique("listed");
    let token = key(&app, "listed", &[&project], AGENT).await;
    for id in ["r1", "r2", "r3", "r4", "r5"] {
        ingest(&app, &project, id).await;
    }
    let set_status = |id: &'static str, status: &'static str| {
        let path = format!("/v1/admin/reports/{project}/{id}");
        let (app, token) = (&app, &token);
        async move {
            let (status, _) =
                call(app, Method::PATCH, &path, token, Some(json!({ "status": status }))).await;
            assert_eq!(status, StatusCode::OK);
        }
    };
    set_status("r2", "triaged").await;
    set_status("r3", "resolved").await;
    set_status("r4", "dismissed").await;
    sqlx::query("UPDATE reports SET kind = 'error', comment = 'Needle in HAY' WHERE project = $1 AND id = 'r5'")
        .bind(&project)
        .execute(&app.db)
        .await
        .unwrap();
    let list = |query: String| {
        let path = format!("/v1/admin/reports?project={project}&{query}");
        let (app, token) = (&app, &token);
        async move { call(app, Method::GET, &path, token, None).await }
    };

    let (status, newest) = list(String::new()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&newest), ["r5", "r4", "r3", "r2", "r1"]);
    assert_eq!(newest["next_cursor"], Value::Null);
    let first = &newest["reports"][4];
    assert_eq!(
        (&first["status"], &first["attachments"], &first["claim"], &first["assignee"]),
        (&json!("new"), &json!(2), &Value::Null, &Value::Null)
    );
    assert_eq!(first.get("envelope"), None);
    assert_eq!(first.get("remote_addr"), None);

    let mut paged = Vec::new();
    let mut cursor = String::new();
    loop {
        let (_, page) = list(format!("limit=2&cursor={cursor}")).await;
        paged.extend(ids(&page).into_iter().map(str::to_owned));
        match page["next_cursor"].as_str() {
            Some(next) => cursor = next.to_owned(),
            None => break,
        }
    }
    assert_eq!(paged, ["r5", "r4", "r3", "r2", "r1"]);

    let second = newest["reports"][3]["received_at"].as_str().unwrap().replace('+', "%2B");
    let (_, since) = list(format!("since={second}")).await;
    assert_eq!(ids(&since), ["r3", "r4", "r5"], "exclusive, oldest first");
    let (_, page) = list(format!("since={second}&limit=2")).await;
    assert_eq!(ids(&page), ["r3", "r4"]);
    let next = page["next_cursor"].as_str().unwrap();
    let (_, page) = list(format!("since={second}&limit=2&cursor={next}")).await;
    assert_eq!((ids(&page), &page["next_cursor"]), (vec!["r5"], &Value::Null));

    assert_eq!(ids(&list("status=open".into()).await.1), ["r5", "r2", "r1"]);
    assert_eq!(ids(&list("status=resolved&status=dismissed".into()).await.1), ["r4", "r3"]);
    assert_eq!(ids(&list("status=triaged,resolved".into()).await.1), ["r3", "r2"]);
    assert_eq!(ids(&list("kind=error".into()).await.1), ["r5"]);
    assert_eq!(ids(&list("q=needle%20in".into()).await.1), ["r5"]);
    assert_eq!(ids(&list("q=R2".into()).await.1), ["r2"]);
    for bad in ["status=bogus", "since=yesterday", "cursor=%25%25", "limit=0"] {
        assert_eq!(list(bad.into()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(ids(&list("limit=9999".into()).await.1).len(), 5);
}

#[tokio::test]
async fn a_patch_changes_fields_and_writes_an_event_for_each() {
    let Some(app) = app().await else { return };
    let project = unique("patched");
    ingest(&app, &project, "r1").await;
    let token = key(&app, "patched", &[&project], AGENT).await;
    let actor = format!("key:{}", unique("patched"));
    let path = format!("/v1/admin/reports/{project}/r1");
    let change = |body: Value| {
        let (app, token, path) = (&app, &token, &path);
        async move { call(app, Method::PATCH, path, token, Some(body)).await }
    };

    let (status, report) =
        change(json!({ "status": "triaged", "assignee": "claude", "ticket": "HMM-2" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&report["status"], &report["assignee"], &report["ticket"]),
        (&json!("triaged"), &json!("claude"), &json!("HMM-2"))
    );
    assert_eq!(event_types(&report), ["received", "status", "assigned", "ticket"]);
    let events = report["events"].as_array().unwrap();
    assert!(events[1..].iter().all(|e| e["actor"] == json!(actor)), "{events:?}");
    assert_eq!(events[1]["data"], json!({ "from": "new", "to": "triaged" }));
    assert_eq!(events[2]["data"], json!({ "from": null, "to": "claude" }));
    assert_eq!(events[3]["data"], json!({ "from": null, "to": "HMM-2" }));

    let (_, report) = change(json!({ "status": "triaged", "assignee": "claude" })).await;
    assert_eq!(event_types(&report).len(), 4, "nothing changed, nothing logged");

    let (_, report) = change(json!({ "assignee": null, "status": "new" })).await;
    assert_eq!((&report["assignee"], &report["ticket"]), (&Value::Null, &json!("HMM-2")));
    assert_eq!(event_types(&report)[4..], ["status", "assigned"]);
    assert_eq!(report["events"][5]["data"], json!({ "from": "claude", "to": null }));

    for (from, to) in
        [("new", "resolved"), ("resolved", "in_progress"), ("in_progress", "dismissed")]
    {
        let (status, report) = change(json!({ "status": to })).await;
        assert_eq!(status, StatusCode::OK);
        let last = report["events"].as_array().unwrap().last().unwrap();
        assert_eq!(last["data"], json!({ "from": from, "to": to }));
    }

    for bad in [json!({ "status": "done" }), json!({ "status": null }), json!({ "ticket": 7 })] {
        assert_eq!(change(bad.clone()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    let garbage = Request::patch(&path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from("{not json"))
        .unwrap();
    assert_eq!(send(&app.public, garbage).await.status, StatusCode::BAD_REQUEST);
    let missing = format!("/v1/admin/reports/{project}/nope");
    let (status, _) =
        call(&app, Method::PATCH, &missing, &token, Some(json!({ "status": "new" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_claim_is_exclusive_until_released_or_expired() {
    let Some(app) = app().await else { return };
    let project = unique("claimed");
    ingest(&app, &project, "r1").await;
    let first = key(&app, "claimed-first", &[&project], AGENT).await;
    let second = key(&app, "claimed-second", &[&project], AGENT).await;
    let first_actor = format!("key:{}", unique("claimed-first"));
    let path = format!("/v1/admin/reports/{project}/r1/claim");
    let claim = |token: &str, body: Option<Value>| {
        let (app, path, token) = (&app, &path, token.to_owned());
        async move { call(app, Method::POST, path, &token, body).await }
    };

    let (status, report) = claim(&first, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["claim"]["by"], json!(first_actor));
    let until = report["claim"]["until"].as_str().unwrap().to_owned();

    let (status, refused) = claim(&second, Some(json!({ "lease_seconds": 60 }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(refused["claim"], json!({ "by": first_actor, "until": until }));
    assert!(refused["error"].is_string());

    let (status, renewed) = claim(&first, Some(json!({ "lease_seconds": 86400 }))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(renewed["claim"]["until"].as_str().unwrap() > until.as_str());

    for seconds in [59, 86401] {
        let (status, _) = claim(&first, Some(json!({ "lease_seconds": seconds }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    assert_eq!(call(&app, Method::DELETE, &path, &second, None).await.0, StatusCode::CONFLICT);
    assert_eq!(call(&app, Method::DELETE, &path, &first, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, Method::DELETE, &path, &first, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(claim(&second, None).await.0, StatusCode::OK);

    sqlx::query(
        "UPDATE reports SET claimed_until = now() - interval '1 second' WHERE project = $1",
    )
    .bind(&project)
    .execute(&app.db)
    .await
    .unwrap();
    let shown = format!("/v1/admin/reports/{project}/r1");
    assert_eq!(call(&app, Method::GET, &shown, &first, None).await.1["claim"], Value::Null);
    assert_eq!(claim(&first, None).await.0, StatusCode::OK, "the lease ended");

    assert_eq!(device(&app, Method::DELETE, &path, None).await.0, StatusCode::NO_CONTENT);
    let (_, report) = call(&app, Method::GET, &shown, &first, None).await;
    assert_eq!(report["claim"], Value::Null);
    assert_eq!(
        event_types(&report),
        ["received", "claimed", "claimed", "released", "claimed", "claimed", "released"]
    );
    let events = report["events"].as_array().unwrap();
    assert!(events[1]["data"]["until"].is_string());
    assert_eq!(events[6]["actor"], "admin:macbook-dorsk");
    assert_eq!(events[6]["data"], json!({}));
}

#[tokio::test]
async fn a_note_is_added_to_the_events() {
    let Some(app) = app().await else { return };
    let project = unique("noted");
    ingest(&app, &project, "r1").await;
    let token = key(&app, "noted", &[&project], AGENT).await;
    let path = format!("/v1/admin/reports/{project}/r1/events");

    let (status, event) =
        call(&app, Method::POST, &path, &token, Some(json!({ "note": "looking" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(event["type"], "note");
    assert_eq!(event["actor"], json!(format!("key:{}", unique("noted"))));
    assert_eq!(event["data"], json!({ "note": "looking" }));
    assert!(event["at"].is_string());

    for bad in [json!({ "note": " " }), json!({ "note": "x".repeat(4001) }), json!({})] {
        let (status, _) = call(&app, Method::POST, &path, &token, Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let missing = format!("/v1/admin/reports/{project}/nope/events");
    let (status, _) =
        call(&app, Method::POST, &missing, &token, Some(json!({ "note": "x" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let shown = format!("/v1/admin/reports/{project}/r1");
    let (_, report) = call(&app, Method::GET, &shown, &token, None).await;
    assert_eq!(event_types(&report), ["received", "note"]);
}

#[tokio::test]
async fn a_screenshot_is_downloaded() {
    let Some(app) = app().await else { return };
    let project = unique("shot");
    ingest(&app, &project, "r1").await;
    let token = key(&app, "shot", &[&project], AGENT).await;
    let path = format!("/v1/admin/reports/{project}/r1/attachments/att-1");

    let reply = send(&app.public, request(Method::GET, &path, Some(&token), None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.headers["content-type"], "image/png");
    assert_eq!(reply.headers["cache-control"], "private, max-age=3600");
    assert_eq!(reply.headers["content-length"], reply.bytes.len().to_string().as_str());
    assert!(reply.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let missing = format!("/v1/admin/reports/{project}/r1/attachments/att-9");
    assert_eq!(call(&app, Method::GET, &missing, &token, None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn key_requests_are_logged_and_old_entries_pruned() {
    let Some(app) = app().await else { return };
    let project = unique("logged");
    ingest(&app, &project, "r1").await;
    let token = key(&app, "logged", &[&project], READ).await;
    let id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM api_keys WHERE name = $1")
        .bind(unique("logged"))
        .fetch_one(&app.db)
        .await
        .unwrap();
    let report = format!("/v1/admin/reports/{project}/r1");

    let mut shown = request(Method::GET, &format!("{report}?x=1"), Some(&token), None);
    shown.headers_mut().insert("x-forwarded-for", "198.51.100.7, 203.0.113.9".parse().unwrap());
    assert_eq!(send(&app.public, shown).await.status, StatusCode::OK);
    call(&app, Method::PATCH, &report, &token, Some(json!({ "status": "triaged" }))).await;
    call(&app, Method::GET, &format!("/v1/admin/reports/{project}/nope"), &token, None).await;
    assert_eq!(device(&app, Method::GET, &report, None).await.0, StatusCode::OK);

    let log = format!("/v1/admin/access?key={id}");
    let (status, access) = device(&app, Method::GET, &log, None).await;
    assert_eq!(status, StatusCode::OK);
    let rows: Vec<(&str, String, i64)> = access["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["method"].as_str().unwrap(),
                e["path"].as_str().unwrap().to_owned(),
                e["status"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("GET", format!("/v1/admin/reports/{project}/nope"), 404),
            ("PATCH", report.clone(), 403),
            ("GET", report.clone(), 200),
        ],
        "newest first, and nothing for the device"
    );
    let oldest = &access["entries"][2];
    assert_eq!(oldest["remote_addr"], "203.0.113.9");
    assert_eq!(oldest["key_id"], json!(id));
    assert_eq!(oldest["key_name"], json!(unique("logged")));
    assert!(oldest["at"].is_string());

    let (_, page) = device(&app, Method::GET, &format!("{log}&limit=2"), None).await;
    assert_eq!(page["entries"].as_array().unwrap().len(), 2);
    let next = page["next_cursor"].as_str().unwrap();
    let (_, page) = device(&app, Method::GET, &format!("{log}&limit=2&cursor={next}"), None).await;
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page["next_cursor"], Value::Null);

    sqlx::query(
        "UPDATE access_log SET at = now() - interval '91 days' \
         WHERE key_id = $1 AND status = 404",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
    assert!(hmm_server::prune_access_log(&app.db).await.unwrap() >= 1);
    let (_, access) = device(&app, Method::GET, &log, None).await;
    assert_eq!(access["entries"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn the_client_certificate_is_believed_only_on_the_admin_listener() {
    let Some(app) = app().await else { return };
    let with_cert = || {
        let mut request = request(Method::GET, "/v1/admin/me", None, None);
        request.headers_mut().insert(auth::XFCC_HEADER, XFCC.parse().unwrap());
        request
    };

    let reply = send(&app.admin, with_cert()).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json!({
            "actor": "admin:macbook-dorsk",
            "scopes": ["reports:read", "attachments:read", "reports:triage", "admin"],
            "projects": ["*"],
        })
    );
    assert_eq!(send(&app.public, with_cert()).await.status, StatusCode::UNAUTHORIZED);

    let bare = request(Method::GET, "/v1/admin/me", None, None);
    assert_eq!(send(&app.admin, bare).await.status, StatusCode::UNAUTHORIZED);
    let mut no_subject = request(Method::GET, "/v1/admin/me", None, None);
    no_subject.headers_mut().insert(auth::XFCC_HEADER, "Hash=abc".parse().unwrap());
    assert_eq!(send(&app.admin, no_subject).await.status, StatusCode::UNAUTHORIZED);

    let token = key(&app, "listener", &["*"], READ).await;
    let reply = send(&app.admin, request(Method::GET, "/v1/admin/me", Some(&token), None)).await;
    assert_eq!(reply.json()["actor"], json!(format!("key:{}", unique("listener"))));
    let mut both = with_cert();
    both.headers_mut().insert("authorization", "Bearer hmm_bad".parse().unwrap());
    assert_eq!(send(&app.admin, both).await.status, StatusCode::UNAUTHORIZED);

    assert_eq!(send(&app.admin, request(Method::GET, "/healthz", None, None)).await.status, 200);
    let ingest = request(Method::POST, "/v1/reports", None, Some(&json!({})));
    assert_eq!(send(&app.admin, ingest).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_admin_api_sends_no_cors_headers() {
    let Some(app) = app().await else { return };
    let token = key(&app, "cors", &["*"], READ).await;
    for router in [&app.public, &app.admin] {
        let mut read = request(Method::GET, "/v1/admin/me", Some(&token), None);
        read.headers_mut().insert("origin", "https://evil.example".parse().unwrap());
        let reply = send(router, read).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(reply.headers.keys().all(|name| !name.as_str().starts_with("access-control")));

        let preflight = Request::options("/v1/admin/me")
            .header("origin", "https://evil.example")
            .header("access-control-request-method", "GET")
            .header("access-control-request-headers", "authorization")
            .body(Body::empty())
            .unwrap();
        let reply = send(router, preflight).await;
        assert!(!reply.status.is_success(), "{}", reply.status);
        assert!(reply.headers.keys().all(|name| !name.as_str().starts_with("access-control")));
    }
}

#[tokio::test]
async fn the_ui_is_served_on_the_admin_listener_only() {
    let dir = std::env::temp_dir().join(unique("hmm-server-ui"));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><title>hmm</title>").unwrap();
    std::fs::write(dir.join("assets/app.js"), "console.log('hmm')").unwrap();
    let Some(app) = app_with(true, Some(&dir)).await else {
        std::fs::remove_dir_all(&dir).unwrap();
        return;
    };
    let get = |router: &Router, path: &str| {
        let (router, request) = (router.clone(), request(Method::GET, path, None, None));
        async move { send(&router, request).await }
    };

    for path in ["/", "/index.html", "/reports/cctui/r_8f2a91", "/assets/missing.js"] {
        let reply = get(&app.admin, path).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert_eq!(reply.headers["content-type"], "text/html", "{path}");
        assert!(reply.bytes.starts_with(b"<!doctype html>"), "{path}");
    }
    let script = get(&app.admin, "/assets/app.js").await;
    assert_eq!(script.status, StatusCode::OK);
    assert_eq!(&script.bytes[..], b"console.log('hmm')");
    assert!(script.headers["content-type"].to_str().unwrap().contains("javascript"));

    for path in ["/v1", "/v1/nope", "/v1/admin/nope", "/../etc/passwd"] {
        let reply = get(&app.admin, path).await;
        assert!(!reply.bytes.starts_with(b"<!doctype html>") || path.starts_with("/.."), "{path}");
        if path.starts_with("/v1") {
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        }
        assert!(!reply.bytes.starts_with(b"root:"), "{path}");
    }
    let posted = send(&app.admin, request(Method::POST, "/", None, None)).await;
    assert_eq!(posted.status, StatusCode::NOT_FOUND);

    for path in ["/", "/index.html", "/assets/app.js"] {
        assert_eq!(get(&app.public, path).await.status, StatusCode::NOT_FOUND, "{path}");
    }
    std::fs::remove_dir_all(&dir).unwrap();

    let Some(bare) = app_with(true, Some(&dir)).await else { return };
    assert_eq!(get(&bare.admin, "/").await.status, StatusCode::NOT_FOUND);
    assert_eq!(get(&bare.admin, "/healthz").await.status, StatusCode::OK);
}

#[derive(Clone)]
struct Hit {
    headers: HeaderMap,
    body: Bytes,
}

impl Hit {
    fn header(&self, name: &str) -> &str {
        self.headers[name].to_str().unwrap()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

#[derive(Clone, Default)]
struct Receiver {
    hits: Arc<Mutex<Vec<Hit>>>,
    script: Arc<Mutex<VecDeque<u16>>>,
}

impl Receiver {
    async fn start(statuses: &[u16]) -> (Self, String) {
        let receiver = Self::default();
        receiver.script.lock().unwrap().extend(statuses);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let router = Router::new()
            .route(
                "/hook",
                axum::routing::post(
                    |State(receiver): State<Self>, headers: HeaderMap, body: Bytes| async move {
                        receiver.hits.lock().unwrap().push(Hit { headers, body });
                        let status = receiver.script.lock().unwrap().pop_front().unwrap_or(200);
                        StatusCode::from_u16(status).unwrap()
                    },
                ),
            )
            .with_state(receiver.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (receiver, url)
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }
}

async fn webhook(app: &App, project: &str, url: &str, events: &[&str]) -> Value {
    let new = json!({ "name": unique("hook"), "project": project, "url": url, "events": events });
    let (status, created) = device(app, Method::POST, "/v1/admin/webhooks", Some(new)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created
}

async fn deliveries(app: &App, webhook: &Value) -> Vec<Value> {
    let path = format!("/v1/admin/deliveries?webhook={}", webhook["id"].as_str().unwrap());
    let (status, body) = device(app, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    body["deliveries"].as_array().unwrap().clone()
}

async fn settle(app: &App, webhook: &Value) -> Vec<Value> {
    for _ in 0..500 {
        webhook::deliver_due(&app.state).await.unwrap();
        let deliveries = deliveries(app, webhook).await;
        if deliveries.iter().all(|d| d["state"] != "pending") {
            return deliveries;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("deliveries did not settle");
}

fn verify(hit: &Hit, secret: &str) {
    let timestamp: i64 = hit.header("x-hmm-timestamp").parse().unwrap();
    assert!((chrono::Utc::now().timestamp() - timestamp).abs() < 300);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(&hit.body);
    let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    assert_eq!(hit.header("x-hmm-signature"), expected);
}

#[tokio::test]
async fn a_webhook_gets_signed_deliveries_for_its_events() {
    let Some(app) = app().await else { return };
    let project = unique("hooked");
    let (receiver, url) = Receiver::start(&[]).await;
    let hook = webhook(&app, &project, &url, &["report.created", "report.status_changed"]).await;
    let secret = hook["secret"].as_str().unwrap().to_owned();
    assert_eq!((&hook["enabled"], &hook["last_delivery"]), (&json!(true), &Value::Null));
    webhook(&app, &unique("hooked-other"), &url, &["report.created"]).await;

    let (stop, stopped) = tokio::sync::watch::channel(());
    let worker = tokio::spawn(webhook::run(app.state.clone(), stopped));
    ingest(&app, &project, "r1").await;
    for _ in 0..500 {
        if !receiver.hits().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(stop);
    worker.await.unwrap();
    let hits = receiver.hits();
    assert_eq!(hits.len(), 1, "one delivery, for the subscribed project only");
    let created = &hits[0];
    verify(created, &secret);
    assert_eq!(created.header("x-hmm-event"), "report.created");
    assert_eq!(created.header("content-type"), "application/json");
    assert_eq!(created.header("user-agent"), concat!("hmm-server/", env!("CARGO_PKG_VERSION")));
    let payload = created.json();
    assert_eq!(payload["delivery_id"], created.header("x-hmm-delivery"));
    assert_eq!(
        (&payload["event"], &payload["project"]),
        (&json!("report.created"), &json!(project))
    );
    assert!(payload["at"].is_string());
    let report = payload["report"].as_object().unwrap();
    let mut fields: Vec<&str> = report.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(fields, ["category", "comment", "id", "kind", "page_url", "received_at", "status"]);
    assert_eq!((&report["id"], &report["status"]), (&json!("r1"), &json!("new")));

    let path = format!("/v1/admin/reports/{project}/r1");
    device(&app, Method::PATCH, &path, Some(json!({ "status": "resolved", "ticket": "X-1" })))
        .await;
    device(&app, Method::PATCH, &path, Some(json!({ "ticket": "X-2" }))).await;
    let settled = settle(&app, &hook).await;
    assert_eq!(settled.len(), 2, "a ticket change is not a webhook event");
    let changed = &settled[0];
    assert_eq!(
        changed,
        &json!({
            "id": changed["id"], "webhook_id": hook["id"], "webhook_name": hook["name"],
            "event": "report.status_changed", "project": project, "report_id": "r1",
            "state": "delivered", "attempts": 1, "next_attempt_at": null,
            "last_status_code": 200, "last_error": null,
            "last_duration_ms": changed["last_duration_ms"],
            "created_at": changed["created_at"], "updated_at": changed["updated_at"],
        })
    );
    let hits = receiver.hits();
    assert_eq!(hits.len(), 2);
    verify(&hits[1], &secret);
    let payload = hits[1].json();
    assert_eq!(
        (&payload["event"], &payload["from"], &payload["to"], &payload["report"]["status"]),
        (&json!("report.status_changed"), &json!("new"), &json!("resolved"), &json!("resolved"))
    );
    assert_eq!(payload["delivery_id"], changed["id"]);

    let (_, report) = device(&app, Method::GET, &path, None).await;
    let logged: Vec<&Value> =
        report["events"].as_array().unwrap().iter().filter(|e| e["type"] == "webhook").collect();
    assert_eq!(logged.len(), 2);
    assert_eq!(logged[0]["actor"], "system");
    assert_eq!(
        logged[0]["data"],
        json!({ "webhook": hook["name"], "event": "report.created", "attempt": 1,
            "status_code": 200, "error": null })
    );

    let (_, listed) = device(&app, Method::GET, "/v1/admin/webhooks", None).await;
    let listed = listed["webhooks"].as_array().unwrap().iter().find(|w| w["id"] == hook["id"]);
    let listed = listed.unwrap();
    assert_eq!(listed.get("secret"), None, "the secret is shown at creation only");
    assert_eq!(listed["last_delivery"]["status_code"], 200);
    assert_eq!(listed["last_delivery"]["error"], Value::Null);
}

#[tokio::test]
async fn a_webhook_is_pinged_edited_redelivered_and_deleted() {
    let Some(app) = app().await else { return };
    let project = unique("managed");
    let (receiver, url) = Receiver::start(&[]).await;
    let hook = webhook(&app, &project, &url, &["report.created"]).await;
    let path = format!("/v1/admin/webhooks/{}", hook["id"].as_str().unwrap());

    let (status, queued) = device(&app, Method::POST, &format!("{path}/ping"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(queued["event"], "ping");
    assert_eq!(queued["report_id"], Value::Null);
    settle(&app, &hook).await;
    let ping = &receiver.hits()[0];
    verify(ping, hook["secret"].as_str().unwrap());
    assert_eq!(ping.header("x-hmm-event"), "ping");
    assert_eq!(
        ping.json(),
        json!({ "event": "ping", "delivery_id": queued["id"], "at": ping.json()["at"],
            "project": project })
    );

    let redeliver = format!("/v1/admin/deliveries/{}/redeliver", queued["id"].as_str().unwrap());
    let (status, again) = device(&app, Method::POST, &redeliver, None).await;
    assert_eq!((status, &again["id"]), (StatusCode::ACCEPTED, &queued["id"]));
    let settled = settle(&app, &hook).await;
    assert_eq!((settled.len(), &settled[0]["attempts"]), (1, &json!(2)));
    assert_eq!(receiver.hits().len(), 2);
    assert_eq!(receiver.hits()[1].header("x-hmm-delivery"), queued["id"].as_str().unwrap());

    let changes =
        json!({ "name": "renamed", "enabled": false, "events": ["report.status_changed"] });
    let (status, edited) = device(&app, Method::PATCH, &path, Some(changes)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&edited["name"], &edited["enabled"], &edited["events"], &edited["url"]),
        (&json!("renamed"), &json!(false), &json!(["report.status_changed"]), &json!(url))
    );
    for bad in [json!({ "events": ["ping"] }), json!({ "url": "http://example.com/" })] {
        assert_eq!(device(&app, Method::PATCH, &path, Some(bad)).await.0, StatusCode::BAD_REQUEST);
    }
    ingest(&app, &project, "r1").await;
    let report = format!("/v1/admin/reports/{project}/r1");
    device(&app, Method::PATCH, &report, Some(json!({ "status": "triaged" }))).await;
    assert_eq!(deliveries(&app, &hook).await.len(), 1, "a disabled webhook queues nothing");

    let narrow = key(&app, "managed-narrow", &[&unique("managed-elsewhere")], &["admin"]).await;
    assert_eq!(call(&app, Method::DELETE, &path, &narrow, None).await.0, StatusCode::NOT_FOUND);
    let (_, seen) = call(&app, Method::GET, "/v1/admin/webhooks", &narrow, None).await;
    assert_eq!(seen, json!({ "webhooks": [] }));

    assert_eq!(device(&app, Method::DELETE, &path, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(device(&app, Method::DELETE, &path, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(device(&app, Method::POST, &redeliver, None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_failed_delivery_is_retried_then_given_up() {
    let Some(app) = app().await else { return };
    let project = unique("retried");
    let (receiver, url) = Receiver::start(&[500]).await;
    let hook = webhook(&app, &project, &url, &["report.created"]).await;
    ingest(&app, &project, "r1").await;

    let settled = settle(&app, &hook).await;
    assert_eq!(settled.len(), 1);
    assert_eq!(
        (&settled[0]["state"], &settled[0]["attempts"], &settled[0]["last_status_code"]),
        (&json!("delivered"), &json!(2), &json!(200))
    );
    let hits = receiver.hits();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].header("x-hmm-delivery"), hits[1].header("x-hmm-delivery"));
    let (_, report) =
        device(&app, Method::GET, &format!("/v1/admin/reports/{project}/r1"), None).await;
    let attempts: Vec<(i64, i64)> = report["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "webhook")
        .map(|e| {
            (e["data"]["attempt"].as_i64().unwrap(), e["data"]["status_code"].as_i64().unwrap())
        })
        .collect();
    assert_eq!(attempts, [(1, 500), (2, 200)]);

    let (dead, url) = Receiver::start(&[503; 10]).await;
    let hopeless = webhook(&app, &project, &url, &["report.created"]).await;
    sqlx::query("UPDATE webhooks SET enabled = false WHERE id = $1::uuid")
        .bind(hook["id"].as_str())
        .execute(&app.db)
        .await
        .unwrap();
    ingest(&app, &project, "r2").await;
    let settled = settle(&app, &hopeless).await;
    assert_eq!(
        (&settled[0]["state"], &settled[0]["attempts"], &settled[0]["last_status_code"]),
        (&json!("failed"), &json!(3), &json!(503)),
        "the first attempt and one retry per backoff step"
    );
    assert_eq!(settled[0]["next_attempt_at"], Value::Null);
    assert_eq!(dead.hits().len(), 3);
    assert_eq!(receiver.hits().len(), 2, "the disabled webhook got nothing more");
}

#[tokio::test]
async fn concurrent_workers_deliver_each_row_once() {
    let Some(app) = app().await else { return };
    let project = unique("raced");
    let (receiver, url) = Receiver::start(&[]).await;
    let hook = webhook(&app, &project, &url, &["report.created"]).await;
    for n in 0..12 {
        ingest(&app, &project, &format!("r{n}")).await;
    }

    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let state = app.state.clone();
        workers.spawn(async move { while webhook::deliver_due(&state).await.unwrap() > 0 {} });
    }
    workers.join_all().await;
    let settled = settle(&app, &hook).await;
    assert_eq!(settled.len(), 12);
    assert!(settled.iter().all(|d| d["state"] == "delivered" && d["attempts"] == 1));

    let mut seen: HashMap<String, usize> = HashMap::new();
    for hit in receiver.hits() {
        *seen.entry(hit.header("x-hmm-delivery").to_owned()).or_default() += 1;
    }
    assert_eq!(seen.len(), 12);
    assert!(seen.values().all(|&count| count == 1), "{seen:?}");
}

#[tokio::test]
async fn webhooks_to_internal_addresses_are_refused() {
    let Some(strict) = app_with(false, None).await else { return };
    let project = unique("guarded");
    let new = |url: &str| json!({ "name": "guarded", "project": project, "url": url, "events": ["report.created"] });
    for url in [
        "http://127.0.0.1:9/hook",
        "https://127.0.0.1/hook",
        "https://localhost/hook",
        "https://169.254.169.254/latest",
        "https://10.0.0.1/hook",
        "https://[::1]/hook",
        "http://1.1.1.1/hook",
        "nonsense",
    ] {
        let (status, body) =
            device(&strict, Method::POST, "/v1/admin/webhooks", Some(new(url))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}");
        assert_eq!(body["error"], "invalid webhook", "{url}");
    }
    let (status, public) =
        device(&strict, Method::POST, "/v1/admin/webhooks", Some(new("https://1.1.1.1/hook")))
            .await;
    assert_eq!(status, StatusCode::CREATED);
    let created = format!("/v1/admin/webhooks/{}", public["id"].as_str().unwrap());
    assert_eq!(device(&strict, Method::DELETE, &created, None).await.0, StatusCode::NO_CONTENT);
    let invalid = json!({ "name": "", "project": "a/b", "url": "https://1.1.1.1/", "events": [] });
    let (status, body) = device(&strict, Method::POST, "/v1/admin/webhooks", Some(invalid)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["details"].as_array().unwrap().len(), 3, "{body}");

    let Some(app) = app().await else { return };
    let (_, allowed) =
        device(&app, Method::POST, "/v1/admin/webhooks", Some(new("https://10.0.0.1/hook"))).await;
    assert_eq!(allowed["error"], "invalid webhook", "only the listed host is exempt");
    ingest(&app, &project, "r1").await;
    let hook: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO webhooks (name, project, url, events, secret) \
         VALUES ('smuggled', $1, 'https://10.0.0.1/hook', '{report.created}', 's') RETURNING id",
    )
    .bind(&project)
    .fetch_one(&app.db)
    .await
    .unwrap();
    ingest(&app, &project, "r2").await;
    let settled = settle(&app, &json!({ "id": hook })).await;
    assert_eq!(settled.len(), 1);
    assert_eq!(
        (&settled[0]["state"], &settled[0]["last_error"], &settled[0]["last_status_code"]),
        (&json!("failed"), &json!("url resolves to a private address"), &Value::Null)
    );
}

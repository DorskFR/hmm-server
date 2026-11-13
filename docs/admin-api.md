# Admin API

Read and triage stored reports, manage API keys and webhooks. Everything is JSON under
`/v1/admin`.

## Listeners

| Bind | Default | Serves |
|---|---|---|
| `HMM_BIND` | `0.0.0.0:8080` | Public: `POST /v1/reports`, `GET /healthz`, `/v1/admin/*` with a bearer key. |
| `HMM_ADMIN_BIND` | `0.0.0.0:8081` | Admin: the web UI at `/`, `/v1/admin/*`, `GET /healthz`. |

On the admin listener a request is authenticated by the client certificate subject the gateway
puts in `x-forwarded-client-cert` (`Subject="CN=<device>,…"`). Such a request has every scope, for
every project, and acts as `admin:<CN>`. A bearer key also works there, and when a request carries
both, the key decides. Without either the answer is `401`. The public listener never reads `x-forwarded-client-cert`.

Run the admin listener only behind a gateway that sets that header itself (sanitising any value the
client sent) and only let the gateway reach the port.

## Authentication

`Authorization: Bearer hmm_<prefix>_<secret>`. `prefix` is 8 characters of `[A-Za-z0-9]`, `secret`
is 32. Only the SHA-256 of the whole token is stored.

| Scope | Allows |
|---|---|
| `reports:read` | List and show reports and their events. |
| `attachments:read` | Download screenshots. |
| `reports:triage` | Change status, assignee, ticket; claim and release; add notes. |
| `admin` | Keys, webhooks, deliveries, access log; `remote_addr` in reports. |

A key lists its projects; `*` means all. A report in a project the key cannot see answers `404`.
A revoked or expired key answers `401`, a missing scope `403`.

Every request authenticated with a key is written to the access log.

## Errors

`{"error": "<message>"}`, with `details: [..]` for validation errors.

## Reports

A report as returned by the list:

```json
{
  "project": "cctui",
  "id": "r_8f2a91",
  "kind": "bug",
  "category": "layout",
  "comment": "…",
  "page_url": "https://cctui.dorsk.dev/sessions/d3cc439d",
  "client_name": "@dorsk/hmm",
  "client_version": "0.2.0",
  "created_at": "2026-11-13T17:06:00Z",
  "received_at": "2026-11-13T17:06:01Z",
  "status": "new",
  "assignee": null,
  "ticket": null,
  "claim": { "by": "key:cctui-agent", "until": "2026-11-13T17:40:00Z" },
  "attachments": 2
}
```

`claim` is `null` when nobody holds it or the lease ended.

### `GET /v1/admin/reports` (`reports:read`)

Query: `project`, `status` (repeatable or comma-separated, or `open` = `new,triaged,in_progress`), `kind`, `q`
(case-insensitive match on comment, page URL and id), `since` (RFC 3339, exclusive, on
`received_at`), `limit` (default 50, max 200), `cursor`.

```json
{ "reports": [ … ], "next_cursor": "opaque or null" }
```

Newest first. With `since` the order is oldest first, so an agent can process in arrival order and
remember the last `received_at`.

### `GET /v1/admin/reports/{project}/{id}` (`reports:read`)

The list fields plus:

```json
{
  "origin": "https://cctui.dorsk.dev",
  "remote_addr": "203.0.113.4",
  "envelope": { … },
  "attachments": [ { "id": "a1", "mime": "image/png", "width": 1440, "height": 900, "bytes": 206112 } ],
  "events": [ { "at": "…", "actor": "system", "type": "received", "data": {} } ]
}
```

`remote_addr` is present only with `admin`. `attachments` here is the list, not the count.

### `GET /v1/admin/reports/{project}/{id}/attachments/{attachment}` (`attachments:read`)

The PNG, `content-type: image/png`, `cache-control: private, max-age=3600`.

### `PATCH /v1/admin/reports/{project}/{id}` (`reports:triage`)

Body: any of `{"status", "assignee", "ticket"}`; `null` (or an empty string) clears assignee or
ticket, which hold at most 200 characters. Statuses:
`new`, `triaged`, `in_progress`, `resolved`, `dismissed`. Any status may move to any other.
Answers the report as `GET` does. Each change writes an event; a value equal to the current one
changes nothing and writes none. An unknown status answers `400`.

### `POST /v1/admin/reports/{project}/{id}/claim` (`reports:triage`)

Body: `{"lease_seconds": 1800}` (60 to 86400, default 1800). Answers `200` with the report when the
caller now holds the claim (renewing its own claim is allowed), `409 {"error", "claim"}` when
someone else holds a live one.

### `DELETE /v1/admin/reports/{project}/{id}/claim` (`reports:triage`)

Releases the caller's claim. An `admin` caller may release anyone's. `204`, also when nothing was
claimed; `409 {"error", "claim"}` when someone else holds it and the caller is not `admin`.

### `POST /v1/admin/reports/{project}/{id}/events` (`reports:triage`)

Body: `{"note": "…"}` (1 to 4000 characters). `201` with the event.

### Events

`{ "at", "actor", "type", "data" }`. `actor` is `system`, `key:<name>` or `admin:<device>`.

| Type | `data` |
|---|---|
| `received` | `{ "attachments": 2, "bytes": 412000 }` |
| `status` | `{ "from": "new", "to": "triaged" }` |
| `assigned` | `{ "from": null, "to": "claude" }` |
| `ticket` | `{ "from": null, "to": "HMM-2" }` |
| `claimed` | `{ "until": "…" }` |
| `released` | `{}` |
| `note` | `{ "note": "…" }` |
| `webhook` | `{ "webhook": "cctui-dispatch", "event": "report.created", "attempt": 1, "status_code": 200, "error": null }` |

## Projects

### `GET /v1/admin/projects` (`reports:read`)

Projects the caller can see that have reports or are in the policy, with counts by status:

```json
{ "projects": [ { "project": "cctui", "counts": { "new": 2, "triaged": 1, "in_progress": 1, "resolved": 0, "dismissed": 1 } } ] }
```

### `GET /v1/admin/me`

```json
{ "actor": "admin:macbook", "scopes": ["admin", "…"], "projects": ["*"] }
```

## Keys (`admin`)

### `GET /v1/admin/keys`

```json
{ "keys": [ { "id": "uuid", "name": "cctui-agent", "prefix": "k7Qp2xAb", "projects": ["cctui"],
  "scopes": ["reports:read"], "created_at": "…", "expires_at": "… or null",
  "revoked_at": null, "last_used_at": "… or null" } ] }
```

### `POST /v1/admin/keys`

Body: `{"name", "projects", "scopes", "expires_at"?}`. `201` with the key plus
`"token": "hmm_<prefix>_<secret>"`. The token is never returned again.

`400` with `details` when a field is invalid, `409` when a key that is not revoked already has the
name (events and claims identify a key by it), `403` when a key asks for projects beyond its own.

### `DELETE /v1/admin/keys/{id}`

Revokes. `204`.

## Access log (`admin`)

### `GET /v1/admin/access?key={id}&cursor=&limit=`

```json
{ "entries": [ { "at", "key_id", "key_name", "method", "path", "status", "remote_addr" } ], "next_cursor": null }
```

Newest first. Entries older than 90 days are deleted.

## Webhooks (`admin`)

```json
{ "id": "uuid", "name": "cctui-dispatch", "project": "cctui", "url": "https://…",
  "events": ["report.created"], "enabled": true, "created_at": "…",
  "last_delivery": { "at": "…", "status_code": 200, "error": null } }
```

| Method | Path | |
|---|---|---|
| GET | `/v1/admin/webhooks` | `{ "webhooks": [ … ] }` |
| POST | `/v1/admin/webhooks` | `{name, project, url, events}` → `201` with the webhook plus `"secret"`, shown once. |
| PATCH | `/v1/admin/webhooks/{id}` | any of `{name, url, events, enabled}` |
| DELETE | `/v1/admin/webhooks/{id}` | `204` |
| POST | `/v1/admin/webhooks/{id}/ping` | Queues a `ping` delivery. `202` with the delivery. |
| GET | `/v1/admin/deliveries?webhook={id}&cursor=&limit=` | `{ "deliveries": [ … ], "next_cursor" }` |
| POST | `/v1/admin/deliveries/{id}/redeliver` | Queues a new attempt of that delivery now. `202` with the delivery. |

A key with `admin` that lists projects sees and manages only the webhooks and deliveries of those
projects; the others answer `404`. A webhook subscribes to `report.created` and
`report.status_changed`.

A delivery:

```json
{ "id": "uuid", "webhook_id": "uuid", "webhook_name": "cctui-dispatch", "event": "report.created",
  "project": "cctui", "report_id": "r_8f2a91", "state": "pending|delivered|failed",
  "attempts": 2, "next_attempt_at": "… or null", "last_status_code": 200, "last_error": null,
  "last_duration_ms": 84, "created_at": "…", "updated_at": "…" }
```

Events: `report.created`, `report.status_changed`, `ping`.

### Delivery

`POST` to the URL with:

```
content-type: application/json
user-agent: hmm-server/<version>
x-hmm-event: report.created
x-hmm-delivery: <delivery id>
x-hmm-timestamp: <unix seconds>
x-hmm-signature: sha256=<hex HMAC-SHA256(secret, "<timestamp>.<body>")>
```

```json
{ "event": "report.created", "delivery_id": "…", "at": "…", "project": "cctui",
  "report": { "id", "kind", "category", "comment", "page_url", "status", "received_at" } }
```

`report.status_changed` adds `"from"` and `"to"`. `ping` has no `report`.

A `2xx` answer delivers it. Anything else, a timeout (10 s) or a connection error is retried after
1 min, 5 min, 30 min, 2 h and 12 h, then the delivery is `failed`. Redirects are not followed.
Disabling a webhook stops new deliveries and fails the ones still waiting for a retry.
Delivery runs in the background and never delays ingest.

URLs must be `https`, except for hosts listed in `HMM_WEBHOOK_ALLOW_HOSTS` (comma-separated), which
may also be `http` and may resolve to private addresses. Other URLs resolving to loopback,
private, link-local or unspecified addresses are refused at creation and at delivery, as is a
host that does not resolve.

## Bootstrap

```sh
hmm-server key create --name cctui-agent --projects cctui \
  --scopes reports:read,attachments:read,reports:triage [--expires-in-days 90]
hmm-server key ls
hmm-server key revoke <prefix>
```

These need only `DATABASE_URL`. `hmm-server` with no subcommand (or `serve`) runs the server.

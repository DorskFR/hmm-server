# hmm-server

Ingest server for [`@dorsk/hmm`](https://github.com/DorskFR/hmm) feedback reports. It checks each
report, scrubs secrets from its text, and stores it: the report in Postgres, the screenshots in an
S3 bucket. An [admin API](#admin-api) with scoped keys lists, shows and triages the stored
reports, and [webhooks](#webhooks) announce new ones.

## Run

```sh
docker run -p 8080:8080 -p 127.0.0.1:8081:8081 \
  -e DATABASE_URL=postgres://hmm:secret@db/hmm \
  -e HMM_S3_BUCKET=hmm -e HMM_S3_ENDPOINT=http://s3:7070 \
  -e AWS_ACCESS_KEY_ID=… -e AWS_SECRET_ACCESS_KEY=… \
  ghcr.io/dorskfr/hmm-server:0.1.0
```

Migrations are applied at start. Point the client at it with `endpoint="https://your-host"`.

| Variable | Default | Purpose |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string. |
| `HMM_S3_BUCKET` | required | Bucket for the screenshots. It must exist. |
| `HMM_S3_ENDPOINT` | AWS | Set for an S3-compatible store; path-style addressing is then used. |
| `HMM_S3_REGION` | `us-east-1` | |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | required | |
| `HMM_BIND` | `0.0.0.0:8080` | Public listener: ingest, and the admin API for bearer keys. |
| `HMM_ADMIN_BIND` | `0.0.0.0:8081` | Admin listener: web UI and admin API. Expose it only to a gateway that authenticates clients, see [Admin API](#admin-api). |
| `HMM_UI_DIR` | `/usr/share/hmm-server/ui` | Built web UI served on the admin listener. Without it `/` answers `404`. |
| `HMM_WEBHOOK_ALLOW_HOSTS` | none | Comma-separated webhook hosts that may use `http` and resolve to private addresses. |
| `HMM_MAX_BODY_BYTES` | 32 MiB | Largest request accepted. |
| `HMM_POLICY` | none | JSON, see [Policy](#policy). |

## HTTP

```
POST /v1/reports
content-type: application/json
x-hmm-key: <project key>

<envelope>
```

| Answer | Meaning |
|---|---|
| `202 {"id"}` | Stored. |
| `200 {"id", "duplicate": true}` | This project already sent a report with this id; nothing was stored again. Clients retry, so this is normal. |
| `400` | Not JSON, not a valid envelope (`details` lists why), the key does not match `project`, or an attachment is not a PNG. |
| `401` | No `x-hmm-key`. |
| `403`, `429`, `507` | Refused by the policy. |
| `413` | Larger than `HMM_MAX_BODY_BYTES`. |

Any origin may call it: the CORS preflight is answered for `content-type` and `x-hmm-key`, and no
credentials are used. `GET /healthz` answers `200` when Postgres is reachable.

A report is checked against [`schema/envelope.v1.json`](schema/envelope.v1.json), the schema
published by the client package (`make schema` refreshes it, and CI fails when it differs from the
published one). Three rules a schema cannot express are checked in code: an item's attachment must
exist, attachment ids are unique, and `context` serialises to at most 16,000 characters.

## Admin API

Everything under `/v1/admin` is specified in [`docs/admin-api.md`](docs/admin-api.md): reports
(list, show, screenshots, status, assignee, ticket, claims, notes), keys, the access log and
webhooks.

A caller sends `Authorization: Bearer hmm_<prefix>_<secret>`. A key is limited to a list of
projects (`*` for all) and to scopes:

| Scope | Allows |
|---|---|
| `reports:read` | List and show reports and their events. |
| `attachments:read` | Download screenshots. |
| `reports:triage` | Change status, assignee, ticket; claim and release; add notes. |
| `admin` | Keys, webhooks, deliveries, access log; `remote_addr` in reports. |

Only the SHA-256 of a key is stored, so the token is shown once, when it is created. The first key
is made on the command line, which needs only `DATABASE_URL`:

```sh
hmm-server key create --name cctui-agent --projects cctui \
  --scopes reports:read,attachments:read,reports:triage [--expires-in-days 90]
hmm-server key ls
hmm-server key revoke <prefix>
```

Every change to a report is kept as an event (`received`, `status`, `assigned`, `ticket`,
`claimed`, `released`, `note`, `webhook`) with who made it, and every request made with a key is
kept in an access log for 90 days.

The admin listener also believes the client certificate subject a gateway puts in
`x-forwarded-client-cert` and gives such a request every scope as `admin:<CN>`. Anyone who can
reach that port can send the header, so only the gateway may reach it, and the gateway must
overwrite whatever value the client sent. The public listener ignores the header, and neither
listener sends CORS headers for `/v1/admin`.

### Polling for new reports

An agent on a timer keeps the `received_at` of the last report it handled and asks for what came
after it. With `since` the list is oldest first, and `since` itself is excluded:

```sh
curl -H "authorization: Bearer $HMM_KEY" \
  "https://your-host/v1/admin/reports?project=cctui&status=new&since=2026-11-13T17:06:01.482913Z"
```

Follow `next_cursor` until it is `null`. Before working on a report, claim it, so that two agents
do not pick the same one:

```sh
curl -X POST -H "authorization: Bearer $HMM_KEY" -d '{"lease_seconds": 1800}' \
  https://your-host/v1/admin/reports/cctui/r_8f2a91/claim
```

`200` means the claim is yours until the lease ends; claim again to extend it. `409` means another
key holds it: skip the report. When done, `PATCH` the status (and `ticket` if one was filed) and
`DELETE` the claim.

## Webhooks

A webhook posts `report.created` and `report.status_changed` for one project to a URL, so an agent
can react without polling. They are managed under `/v1/admin/webhooks` with the `admin` scope; the
signing secret is shown once, at creation. Deliveries are queued in the transaction that stores the
change and sent in the background, so a slow receiver never delays ingest; failures are retried
after 1 min, 5 min, 30 min, 2 h and 12 h. Several instances may run against one database without
delivering twice.

Each request carries `x-hmm-timestamp` (Unix seconds) and
`x-hmm-signature: sha256=<hex>`, the HMAC-SHA256 of `<timestamp>.<raw body>` keyed with the
secret. A receiver recomputes it over the bytes it received, compares in constant time, and
refuses timestamps older than five minutes to stop replays:

```js
import { createHmac, timingSafeEqual } from 'node:crypto';

function verify(secret, headers, rawBody) {
  const timestamp = headers['x-hmm-timestamp'];
  if (Math.abs(Date.now() / 1000 - Number(timestamp)) > 300) return false;
  const expected = 'sha256=' + createHmac('sha256', secret).update(`${timestamp}.${rawBody}`).digest('hex');
  const given = Buffer.from(headers['x-hmm-signature'] ?? '');
  return given.length === expected.length && timingSafeEqual(given, Buffer.from(expected));
}
```

A delivery may arrive more than once (a retry after a lost answer); `x-hmm-delivery` is the same
each time. The payload carries no screenshots and no client address: fetch the report with a key.
URLs must be `https` and must not resolve to loopback, private or link-local addresses, except for
hosts in `HMM_WEBHOOK_ALLOW_HOSTS`.

## Policy

Without `HMM_POLICY` every valid report is accepted, for any project key. Each field is optional
and adds one limit:

```json
{
  "projects": {
    "shop": { "origins": ["https://shop.example"] },
    "docs": {}
  },
  "rate_per_minute": 30,
  "storage_budget_bytes": 5000000000
}
```

- `projects`: only these keys are accepted. With `origins`, the request `Origin` must be listed.
- `rate_per_minute`: per project key and client address, counted in memory per instance. The
  address is the last entry of `X-Forwarded-For`, so run it behind one trusted proxy.
- `storage_budget_bytes`: once the stored screenshots reach this total, new reports get `507`.

The project key is public, as it ships in the page. `Origin` can be forged outside a browser, so
these limit casual misuse, not a determined sender.

## Data

- `reports`: one row per report, keyed by `(project, id)`. Columns for the fields worth querying,
  the triage state (`status`, `assignee`, `ticket`, the current claim), and `envelope`: the
  scrubbed report as JSON, where each attachment carries `key` and `bytes` in place of the image
  data.
- `report_events`: what happened to each report, and by whom.
- `api_keys`, `access_log`: the keys (hashed) and the requests made with them.
- `webhooks`, `webhook_deliveries`: the webhooks and each delivery with its attempts.
- `attachments`: one row per screenshot with its size and `object_key`.
- Objects are stored as `<project>/<report id>/<attachment id>.png`.

Text is scrubbed of JWTs, bearer tokens, common API key formats, private keys and
`password=`-style pairs, and URLs lose credentials and secret-looking query parameters, with the
same rules as the client. Screenshots cannot be scrubbed. Treat every stored field as untrusted
input.

## Development

```sh
make db/up    # throwaway Postgres on 127.0.0.1:5499
make test
make lint
make fmt
make db/down
```

MIT licensed.

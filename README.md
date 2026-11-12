# hmm-server

Ingest server for [`@dorsk/hmm`](https://github.com/DorskFR/hmm) feedback reports. It checks each
report, scrubs secrets from its text, and stores it: the report in Postgres, the screenshots in an
S3 bucket. Reading and triaging the stored reports is left to whatever consumes the database.

## Run

```sh
docker run -p 8080:8080 \
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
| `HMM_BIND` | `0.0.0.0:8080` | |
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
  a `status` that starts at `new` for consumers to move along, and `envelope`: the scrubbed report
  as JSON, where each attachment carries `key` and `bytes` in place of the image data.
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

ALTER TABLE reports
    ADD COLUMN assignee      text,
    ADD COLUMN ticket        text,
    ADD COLUMN claimed_by    text,
    ADD COLUMN claimed_until timestamptz;

CREATE INDEX reports_received_idx ON reports (received_at, project, id);

CREATE TABLE api_keys (
    id           uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    name         text        NOT NULL,
    prefix       text        NOT NULL UNIQUE,
    -- SHA-256 of the whole token, hex.
    hash         text        NOT NULL UNIQUE,
    projects     text[]      NOT NULL,
    scopes       text[]      NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz,
    revoked_at   timestamptz,
    last_used_at timestamptz
);

-- Events and claims name a key by `key:<name>`, so live names are unique.
CREATE UNIQUE INDEX api_keys_live_name_idx ON api_keys (name) WHERE revoked_at IS NULL;

CREATE TABLE report_events (
    id        bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project   text        NOT NULL,
    report_id text        NOT NULL,
    at        timestamptz NOT NULL DEFAULT now(),
    actor     text        NOT NULL,
    type      text        NOT NULL,
    data      jsonb       NOT NULL DEFAULT '{}',
    FOREIGN KEY (project, report_id) REFERENCES reports (project, id) ON DELETE CASCADE
);

CREATE INDEX report_events_report_idx ON report_events (project, report_id, id);

INSERT INTO report_events (project, report_id, at, actor, type, data)
SELECT r.project, r.id, r.received_at, 'system', 'received',
       jsonb_build_object('attachments', count(a.id), 'bytes', COALESCE(sum(a.bytes), 0)::bigint)
FROM reports r
LEFT JOIN attachments a ON a.project = r.project AND a.report_id = r.id
GROUP BY r.project, r.id, r.received_at;

CREATE TABLE access_log (
    id          bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at          timestamptz NOT NULL DEFAULT now(),
    key_id      uuid        NOT NULL REFERENCES api_keys (id) ON DELETE CASCADE,
    method      text        NOT NULL,
    path        text        NOT NULL,
    status      integer     NOT NULL,
    remote_addr text
);

CREATE INDEX access_log_at_idx ON access_log (at);
CREATE INDEX access_log_key_idx ON access_log (key_id, id);

CREATE TABLE webhooks (
    id         uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    name       text        NOT NULL,
    project    text        NOT NULL,
    url        text        NOT NULL,
    events     text[]      NOT NULL,
    -- Kept in clear: it signs every delivery.
    secret     text        NOT NULL,
    enabled    boolean     NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX webhooks_project_idx ON webhooks (project);

-- One row per delivery; pending rows are the outbox the worker drains.
CREATE TABLE webhook_deliveries (
    id               uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    webhook_id       uuid        NOT NULL REFERENCES webhooks (id) ON DELETE CASCADE,
    event            text        NOT NULL,
    project          text        NOT NULL,
    report_id        text,
    payload          jsonb       NOT NULL,
    state            text        NOT NULL DEFAULT 'pending',
    attempts         integer     NOT NULL DEFAULT 0,
    next_attempt_at  timestamptz DEFAULT now(),
    last_status_code integer,
    last_error       text,
    last_duration_ms integer,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (project, report_id) REFERENCES reports (project, id) ON DELETE CASCADE
);

CREATE INDEX webhook_deliveries_due_idx ON webhook_deliveries (next_attempt_at) WHERE state = 'pending';
CREATE INDEX webhook_deliveries_webhook_idx ON webhook_deliveries (webhook_id, created_at DESC, id DESC);
CREATE INDEX webhook_deliveries_created_idx ON webhook_deliveries (created_at DESC, id DESC);

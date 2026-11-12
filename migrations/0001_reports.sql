CREATE TABLE reports (
    project        text        NOT NULL,
    id             text        NOT NULL,
    kind           text        NOT NULL,
    category       text,
    fingerprint    text,
    comment        text        NOT NULL,
    page_url       text,
    client_name    text        NOT NULL,
    client_version text        NOT NULL,
    created_at     timestamptz NOT NULL,
    received_at    timestamptz NOT NULL DEFAULT now(),
    origin         text,
    remote_addr    text,
    status         text        NOT NULL DEFAULT 'new',
    -- The report as received, scrubbed, with attachment image data removed.
    envelope       jsonb       NOT NULL,
    PRIMARY KEY (project, id)
);

CREATE INDEX reports_status_received_idx ON reports (status, received_at);
CREATE INDEX reports_project_received_idx ON reports (project, received_at DESC);

CREATE TABLE attachments (
    project    text    NOT NULL,
    report_id  text    NOT NULL,
    id         text    NOT NULL,
    mime       text    NOT NULL,
    width      integer NOT NULL,
    height     integer NOT NULL,
    bytes      bigint  NOT NULL,
    object_key text    NOT NULL,
    PRIMARY KEY (project, report_id, id),
    FOREIGN KEY (project, report_id) REFERENCES reports (project, id) ON DELETE CASCADE
);

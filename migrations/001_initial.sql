-- Upload metadata
CREATE TABLE IF NOT EXISTS uploads (
    upload_id         TEXT PRIMARY KEY,
    root_cid          TEXT NOT NULL,
    source_ip_hash    TEXT NOT NULL,
    auth_mode         TEXT NOT NULL CHECK (auth_mode IN ('anonymous', 'api_key')),
    bytes             INTEGER NOT NULL,
    file_count        INTEGER NOT NULL,
    created_at        DATETIME NOT NULL,
    request_id        TEXT NOT NULL,
    replication_status TEXT NOT NULL DEFAULT 'pending'
);

CREATE INDEX IF NOT EXISTS idx_uploads_created_at ON uploads (created_at);

-- Async replication jobs
CREATE TABLE IF NOT EXISTS replication_jobs (
    id          TEXT PRIMARY KEY,
    upload_id   TEXT NOT NULL REFERENCES uploads(upload_id),
    cid         TEXT NOT NULL,
    target      TEXT NOT NULL,  -- 'pinata' | '4everland'
    status      TEXT NOT NULL CHECK (status IN ('queued', 'pinned', 'failed')) DEFAULT 'queued',
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    created_at  DATETIME NOT NULL,
    updated_at  DATETIME
);

CREATE INDEX IF NOT EXISTS idx_replication_jobs_status ON replication_jobs (status, attempts, created_at);
CREATE INDEX IF NOT EXISTS idx_replication_jobs_upload ON replication_jobs (upload_id);

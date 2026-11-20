-- Golden schema from M0 commit 4fdada3; do not regenerate from current migrations.

CREATE TABLE IF NOT EXISTS process_versions (
    version TEXT PRIMARY KEY,
    manifest TEXT NOT NULL,
    canonical_format TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    request_key TEXT NOT NULL UNIQUE,
    request_digest TEXT NOT NULL,
    version TEXT NOT NULL REFERENCES process_versions(version),
    input TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0,
    output TEXT,
    error TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS steps (
    run_id TEXT NOT NULL REFERENCES runs(id),
    id TEXT NOT NULL,
    position INTEGER NOT NULL,
    state TEXT NOT NULL,
    output TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    active_attempt TEXT,
    fence INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, id)
);
CREATE TABLE IF NOT EXISTS attempts (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    number INTEGER NOT NULL,
    fence INTEGER NOT NULL,
    state TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    error TEXT,
    FOREIGN KEY (run_id, step_id) REFERENCES steps(run_id, id),
    UNIQUE (run_id, step_id, number)
);
CREATE TABLE IF NOT EXISTS handoffs (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    input TEXT NOT NULL,
    result_schema TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    state TEXT NOT NULL,
    FOREIGN KEY (run_id, step_id) REFERENCES steps(run_id, id),
    UNIQUE (run_id, step_id, generation)
);
CREATE TABLE IF NOT EXISTS receipts (
    run_id TEXT NOT NULL REFERENCES runs(id),
    submission_key TEXT NOT NULL,
    semantic_digest TEXT NOT NULL,
    receipt TEXT NOT NULL,
    PRIMARY KEY (run_id, submission_key)
);
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    reference TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS artifact_links (
    run_id TEXT NOT NULL REFERENCES runs(id),
    step_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id),
    PRIMARY KEY (run_id, step_id, artifact_id)
);
CREATE TABLE IF NOT EXISTS events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES runs(id),
    kind TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS events_by_run ON events(run_id, sequence);
CREATE INDEX IF NOT EXISTS handoffs_by_run ON handoffs(run_id, state);
PRAGMA user_version = 1;

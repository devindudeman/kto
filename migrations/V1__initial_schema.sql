-- kto initial database schema

-- Watches table: stores watch configurations
CREATE TABLE watches (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    engine TEXT NOT NULL DEFAULT 'http',
    extraction TEXT NOT NULL, -- JSON
    normalization TEXT NOT NULL, -- JSON
    filters TEXT NOT NULL DEFAULT '[]', -- JSON array
    agent_config TEXT, -- JSON, nullable
    interval_secs INTEGER NOT NULL DEFAULT 900,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    headers TEXT NOT NULL DEFAULT '{}', -- JSON
    cookie_file TEXT,
    storage_state TEXT
);

-- Snapshots table: stores page content at points in time
CREATE TABLE snapshots (
    id TEXT PRIMARY KEY,
    watch_id TEXT NOT NULL REFERENCES watches(id) ON DELETE CASCADE,
    fetched_at INTEGER NOT NULL,
    raw_html BLOB, -- zstd compressed, kept for last 5
    extracted TEXT NOT NULL, -- normalized content
    content_hash TEXT NOT NULL
);

CREATE INDEX idx_snapshots_watch_id ON snapshots(watch_id);
CREATE INDEX idx_snapshots_fetched_at ON snapshots(fetched_at);

-- Changes table: records detected changes between snapshots
CREATE TABLE changes (
    id TEXT PRIMARY KEY,
    watch_id TEXT NOT NULL REFERENCES watches(id) ON DELETE CASCADE,
    detected_at INTEGER NOT NULL,
    old_snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
    new_snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
    diff TEXT NOT NULL,
    filter_passed INTEGER NOT NULL DEFAULT 1,
    agent_response TEXT, -- JSON, nullable
    notified INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_changes_watch_id ON changes(watch_id);
CREATE INDEX idx_changes_detected_at ON changes(detected_at);

-- Agent memory table: per-watch AI memory
CREATE TABLE agent_memory (
    watch_id TEXT PRIMARY KEY REFERENCES watches(id) ON DELETE CASCADE,
    memory TEXT NOT NULL DEFAULT '{}', -- JSON
    updated_at INTEGER NOT NULL
);

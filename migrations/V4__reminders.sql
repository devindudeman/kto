-- Reminders table for simple scheduled notifications
CREATE TABLE IF NOT EXISTS reminders (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    message TEXT,
    trigger_at INTEGER NOT NULL,
    interval_secs INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    notify_target TEXT,
    created_at INTEGER NOT NULL
);

-- Index for efficient due reminder lookup
CREATE INDEX IF NOT EXISTS idx_reminders_trigger ON reminders(trigger_at, enabled);

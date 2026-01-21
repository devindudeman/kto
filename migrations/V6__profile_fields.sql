-- Add use_profile column to watches table (opt-in per watch)
ALTER TABLE watches ADD COLUMN use_profile INTEGER DEFAULT 0;

-- Create global_memory table for cross-watch learning
CREATE TABLE IF NOT EXISTS global_memory (
    id INTEGER PRIMARY KEY,
    memory_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

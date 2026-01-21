-- Migration: Add per-watch notification target
-- Allows watches to override the global notification setting

ALTER TABLE watches ADD COLUMN notify_target TEXT; -- JSON, nullable

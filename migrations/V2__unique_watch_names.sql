-- Migration: Enforce unique watch names
-- This migration deduplicates existing watch names and adds a unique constraint

-- Step 1: Fix any existing duplicate names by appending _2, _3, etc.
-- SQLite doesn't have row_number(), so we use a recursive approach with UPDATE
-- We update duplicate names in a way that preserves the first (oldest) watch

-- First, create a temp table to identify duplicates
CREATE TEMP TABLE _dupe_watches AS
SELECT id, name, created_at,
       (SELECT COUNT(*) FROM watches w2 WHERE w2.name = watches.name AND w2.created_at < watches.created_at) as dupe_rank
FROM watches
WHERE name IN (SELECT name FROM watches GROUP BY name HAVING COUNT(*) > 1);

-- Update duplicates with suffixes based on their rank
UPDATE watches
SET name = name || '_' || (
    SELECT dupe_rank + 1 FROM _dupe_watches WHERE _dupe_watches.id = watches.id
)
WHERE id IN (SELECT id FROM _dupe_watches WHERE dupe_rank > 0);

DROP TABLE _dupe_watches;

-- Step 2: Add unique index to prevent future duplicates
CREATE UNIQUE INDEX idx_watches_name_unique ON watches(name);

-- Add tags column to watches table
ALTER TABLE watches ADD COLUMN tags TEXT DEFAULT '[]';

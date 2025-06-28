-- Rollback migration 0006: Remove file-level content addressable storage
-- This migration removes the file-level hash tracking tables

-- Drop indexes first
DROP INDEX IF EXISTS idx_packages_has_file_hashes;
DROP INDEX IF EXISTS idx_file_verification_cache_verified_at;
DROP INDEX IF EXISTS idx_installed_files_path;
DROP INDEX IF EXISTS idx_installed_files_file_hash;
DROP INDEX IF EXISTS idx_installed_files_package_id;
DROP INDEX IF EXISTS idx_installed_files_state_id;
DROP INDEX IF EXISTS idx_package_file_entries_path;
DROP INDEX IF EXISTS idx_package_file_entries_file_hash;
DROP INDEX IF EXISTS idx_package_file_entries_package_id;
DROP INDEX IF EXISTS idx_file_objects_ref_count;
DROP INDEX IF EXISTS idx_file_objects_created_at;
DROP INDEX IF EXISTS idx_file_objects_size;

-- Drop tables
DROP TABLE IF EXISTS file_verification_cache;
DROP TABLE IF EXISTS installed_files;
DROP TABLE IF EXISTS package_file_entries;
DROP TABLE IF EXISTS file_objects;

-- Remove columns from packages table
-- Note: SQLite doesn't support DROP COLUMN directly, so we need to recreate the table
-- This is a more complex operation that should be done carefully in production

-- Create temporary table with original schema
CREATE TABLE packages_temp (
    id INTEGER PRIMARY KEY,
    state_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    hash TEXT NOT NULL,
    size INTEGER NOT NULL,
    installed_at INTEGER NOT NULL,
    venv_path TEXT,
    FOREIGN KEY (state_id) REFERENCES states(id),
    UNIQUE(state_id, name)
);

-- Copy data from current packages table
INSERT INTO packages_temp (id, state_id, name, version, hash, size, installed_at, venv_path)
SELECT id, state_id, name, version, hash, size, installed_at, venv_path
FROM packages;

-- Drop the current packages table
DROP TABLE packages;

-- Rename temp table to packages
ALTER TABLE packages_temp RENAME TO packages;

-- Recreate indexes
CREATE INDEX idx_packages_state_id ON packages(state_id);
CREATE INDEX idx_packages_name ON packages(name);
CREATE INDEX idx_packages_hash ON packages(hash);

-- Update schema version
UPDATE schema_version SET version = 5, applied_at = strftime('%s', 'now');
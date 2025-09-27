-- Initial schema for sps2 state management

-- States table
CREATE TABLE states (
    id TEXT PRIMARY KEY,
    parent_id TEXT,
    created_at INTEGER NOT NULL,
    operation TEXT NOT NULL,
    success BOOLEAN NOT NULL DEFAULT 1,
    rollback_of TEXT,
    FOREIGN KEY (parent_id) REFERENCES states(id),
    FOREIGN KEY (rollback_of) REFERENCES states(id)
);

CREATE INDEX idx_states_created_at ON states(created_at);
CREATE INDEX idx_states_parent_id ON states(parent_id);

-- Active state tracking
CREATE TABLE active_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    state_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (state_id) REFERENCES states(id)
);

-- Package installations
CREATE TABLE packages (
    id INTEGER PRIMARY KEY,
    state_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    hash TEXT NOT NULL,
    size INTEGER NOT NULL,
    installed_at INTEGER NOT NULL,
    FOREIGN KEY (state_id) REFERENCES states(id),
    UNIQUE(state_id, name)
);

CREATE INDEX idx_packages_state_id ON packages(state_id);
CREATE INDEX idx_packages_name ON packages(name);
CREATE INDEX idx_packages_hash ON packages(hash);

-- Package dependencies
CREATE TABLE dependencies (
    id INTEGER PRIMARY KEY,
    package_id INTEGER NOT NULL,
    dep_name TEXT NOT NULL,
    dep_spec TEXT NOT NULL,
    dep_kind TEXT NOT NULL CHECK (dep_kind IN ('runtime', 'build')),
    FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE
);

CREATE INDEX idx_dependencies_package_id ON dependencies(package_id);

-- Store reference counting
CREATE TABLE store_refs (
    hash TEXT PRIMARY KEY,
    ref_count INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

-- Garbage collection log
CREATE TABLE gc_log (
    id INTEGER PRIMARY KEY,
    run_at INTEGER NOT NULL,
    items_removed INTEGER NOT NULL,
    space_freed INTEGER NOT NULL
);

-- Schema version tracking
CREATE TABLE schema_version (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);

INSERT INTO schema_version (version, applied_at) VALUES (1, strftime('%s', 'now'));
-- Add build dependencies tracking

-- Build environments table
CREATE TABLE build_envs (
    id TEXT PRIMARY KEY,
    package_name TEXT NOT NULL,
    package_version TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    destroyed_at INTEGER
);

CREATE INDEX idx_build_envs_package ON build_envs(package_name, package_version);

-- Build dependencies installed in environments
CREATE TABLE build_env_deps (
    id INTEGER PRIMARY KEY,
    env_id TEXT NOT NULL,
    dep_name TEXT NOT NULL,
    dep_version TEXT NOT NULL,
    dep_hash TEXT NOT NULL,
    FOREIGN KEY (env_id) REFERENCES build_envs(id) ON DELETE CASCADE
);

CREATE INDEX idx_build_env_deps_env_id ON build_env_deps(env_id);

-- Update schema version
UPDATE schema_version SET version = 2, applied_at = strftime('%s', 'now');
-- Add package file tracking for uninstall support

-- Package files table to track which files belong to which packages
CREATE TABLE package_files (
    id INTEGER PRIMARY KEY,
    state_id TEXT NOT NULL,
    package_name TEXT NOT NULL,
    package_version TEXT NOT NULL,
    file_path TEXT NOT NULL,
    is_directory BOOLEAN NOT NULL DEFAULT 0,
    FOREIGN KEY (state_id) REFERENCES states(id) ON DELETE CASCADE
);

CREATE INDEX idx_package_files_state_id ON package_files(state_id);
CREATE INDEX idx_package_files_package ON package_files(state_id, package_name, package_version);
CREATE INDEX idx_package_files_path ON package_files(state_id, file_path);

-- Update schema version
UPDATE schema_version SET version = 3, applied_at = strftime('%s', 'now');
-- Add Python virtual environment tracking

-- Add venv_path column to packages table
ALTER TABLE packages ADD COLUMN venv_path TEXT;

-- Create index for packages with venvs
CREATE INDEX idx_packages_venv ON packages(state_id, name, version) WHERE venv_path IS NOT NULL;

-- Update schema version
UPDATE schema_version SET version = 4, applied_at = strftime('%s', 'now');
-- Add package_map table for name/version to hash mapping
CREATE TABLE IF NOT EXISTS package_map (
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    hash TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (name, version),
    FOREIGN KEY (hash) REFERENCES store_refs(hash)
);

-- Create index for hash lookups
CREATE INDEX idx_package_map_hash ON package_map(hash);

-- Create index for name lookups
CREATE INDEX idx_package_map_name ON package_map(name);
-- Migration 0006: Add file-level content addressable storage
-- This migration adds support for tracking individual file hashes within packages

-- Table 1: Content-addressed file storage
CREATE TABLE file_objects (
    hash TEXT PRIMARY KEY,              -- BLAKE3 hash of file content
    size INTEGER NOT NULL,              -- File size in bytes
    created_at INTEGER NOT NULL,        -- Unix timestamp of first occurrence
    ref_count INTEGER NOT NULL DEFAULT 0, -- Reference count for garbage collection
    is_executable BOOLEAN NOT NULL DEFAULT 0, -- Executable flag
    is_symlink BOOLEAN NOT NULL DEFAULT 0,    -- Symlink flag
    symlink_target TEXT,                -- Target path for symlinks
    CHECK (
        (is_symlink = 1 AND symlink_target IS NOT NULL) OR
        (is_symlink = 0 AND symlink_target IS NULL)
    )
);

CREATE INDEX idx_file_objects_size ON file_objects(size);
CREATE INDEX idx_file_objects_created_at ON file_objects(created_at);
CREATE INDEX idx_file_objects_ref_count ON file_objects(ref_count) WHERE ref_count > 0;

-- Table 2: Files within packages
CREATE TABLE package_file_entries (
    id INTEGER PRIMARY KEY,
    package_id INTEGER NOT NULL,        -- References packages(id)
    file_hash TEXT NOT NULL,            -- References file_objects(hash)
    relative_path TEXT NOT NULL,        -- Path within package (normalized)
    permissions INTEGER NOT NULL,       -- Unix permissions (mode)
    uid INTEGER NOT NULL DEFAULT 0,     -- User ID (for future use)
    gid INTEGER NOT NULL DEFAULT 0,     -- Group ID (for future use)
    mtime INTEGER,                      -- Modification time (optional)
    FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
    FOREIGN KEY (file_hash) REFERENCES file_objects(hash),
    UNIQUE(package_id, relative_path)   -- One file per path per package
);

CREATE INDEX idx_package_file_entries_package_id ON package_file_entries(package_id);
CREATE INDEX idx_package_file_entries_file_hash ON package_file_entries(file_hash);
CREATE INDEX idx_package_file_entries_path ON package_file_entries(relative_path);

-- Table 3: Tracking installed file locations
CREATE TABLE installed_files (
    id INTEGER PRIMARY KEY,
    state_id TEXT NOT NULL,             -- References states(id)
    package_id INTEGER NOT NULL,        -- References packages(id)
    file_hash TEXT NOT NULL,            -- References file_objects(hash)
    installed_path TEXT NOT NULL,       -- Absolute path on filesystem
    is_directory BOOLEAN NOT NULL DEFAULT 0,
    FOREIGN KEY (state_id) REFERENCES states(id) ON DELETE CASCADE,
    FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
    FOREIGN KEY (file_hash) REFERENCES file_objects(hash),
    UNIQUE(state_id, installed_path)    -- One file per path per state
);

CREATE INDEX idx_installed_files_state_id ON installed_files(state_id);
CREATE INDEX idx_installed_files_package_id ON installed_files(package_id);
CREATE INDEX idx_installed_files_file_hash ON installed_files(file_hash);
CREATE INDEX idx_installed_files_path ON installed_files(installed_path);

-- Table 4: File modification time tracking for verification optimization
CREATE TABLE file_mtime_tracker (
    file_path TEXT PRIMARY KEY,
    last_verified_mtime INTEGER NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- Index for faster lookups
CREATE INDEX idx_file_mtime_tracker_path ON file_mtime_tracker(file_path);
CREATE INDEX idx_file_mtime_tracker_mtime ON file_mtime_tracker(last_verified_mtime);

-- Trigger to update the updated_at timestamp
CREATE TRIGGER update_file_mtime_tracker_updated_at
    AFTER UPDATE ON file_mtime_tracker
    FOR EACH ROW
BEGIN
    UPDATE file_mtime_tracker
    SET updated_at = strftime('%s', 'now')
    WHERE file_path = NEW.file_path;
END;

-- Modify packages table to support file-level hashing
ALTER TABLE packages ADD COLUMN computed_hash TEXT;
ALTER TABLE packages ADD COLUMN has_file_hashes BOOLEAN NOT NULL DEFAULT 0;

-- Index for packages with file-level hashes
CREATE INDEX idx_packages_has_file_hashes ON packages(has_file_hashes) WHERE has_file_hashes = 1;

-- Update schema version
UPDATE schema_version SET version = 6, applied_at = strftime('%s', 'now');
-- Migration 0007: Add store verification tracking
-- This migration adds columns to track verification status for content-addressed store objects

-- Add verification tracking columns to file_objects table
ALTER TABLE file_objects ADD COLUMN last_verified_at INTEGER;
ALTER TABLE file_objects ADD COLUMN verification_status TEXT NOT NULL DEFAULT 'pending' CHECK (
    verification_status IN ('pending', 'verified', 'failed', 'quarantined')
);
ALTER TABLE file_objects ADD COLUMN verification_error TEXT;
ALTER TABLE file_objects ADD COLUMN verification_attempts INTEGER NOT NULL DEFAULT 0;

-- Indexes for efficient verification queries
CREATE INDEX idx_file_objects_verification_status ON file_objects(verification_status);
CREATE INDEX idx_file_objects_last_verified_at ON file_objects(last_verified_at);
CREATE INDEX idx_file_objects_verification_pending ON file_objects(verification_status, last_verified_at) 
    WHERE verification_status = 'pending';

-- Index for finding objects that need re-verification (older than threshold)
CREATE INDEX idx_file_objects_needs_reverification ON file_objects(last_verified_at, verification_status)
    WHERE verification_status = 'verified';

-- Index for failed verification tracking
CREATE INDEX idx_file_objects_verification_failed ON file_objects(verification_status, verification_attempts)
    WHERE verification_status = 'failed';

-- Update schema version
UPDATE schema_version SET version = 7, applied_at = strftime('%s', 'now');
-- CAS eviction audit tables

CREATE TABLE IF NOT EXISTS package_evictions (
    hash TEXT PRIMARY KEY,
    evicted_at INTEGER NOT NULL,
    size INTEGER NOT NULL,
    reason TEXT
);

CREATE TABLE IF NOT EXISTS file_object_evictions (
    hash TEXT PRIMARY KEY,
    evicted_at INTEGER NOT NULL,
    size INTEGER NOT NULL,
    reason TEXT
);

CREATE INDEX IF NOT EXISTS idx_package_evictions_time ON package_evictions(evicted_at);
CREATE INDEX IF NOT EXISTS idx_file_object_evictions_time ON file_object_evictions(evicted_at);

-- Update schema version (best-effort; ignore if table doesn't exist)
INSERT OR REPLACE INTO schema_version (version, applied_at)
    VALUES (8, strftime('%s', 'now'));


-- Add pruning marker to states to control base history visibility

ALTER TABLE states ADD COLUMN pruned_at INTEGER NULL;

CREATE INDEX IF NOT EXISTS idx_states_pruned_at ON states(pruned_at);

-- Bump schema version
INSERT OR REPLACE INTO schema_version (version, applied_at)
    VALUES (9, strftime('%s', 'now'));


-- Track package archive hashes alongside store hashes

ALTER TABLE package_map ADD COLUMN package_hash TEXT;

CREATE INDEX IF NOT EXISTS idx_package_map_package_hash
    ON package_map(package_hash);

-- Bump schema version
INSERT OR REPLACE INTO schema_version (version, applied_at)
    VALUES (10, strftime('%s', 'now'));


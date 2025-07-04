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
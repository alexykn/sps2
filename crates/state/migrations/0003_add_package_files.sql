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
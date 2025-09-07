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


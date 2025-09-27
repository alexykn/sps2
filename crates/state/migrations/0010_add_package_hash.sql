-- Track package archive hashes alongside store hashes

ALTER TABLE package_map ADD COLUMN package_hash TEXT;

CREATE INDEX IF NOT EXISTS idx_package_map_package_hash
    ON package_map(package_hash);

-- Bump schema version
INSERT OR REPLACE INTO schema_version (version, applied_at)
    VALUES (10, strftime('%s', 'now'));

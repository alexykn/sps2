-- Add pruning marker to states to control base history visibility

ALTER TABLE states ADD COLUMN pruned_at INTEGER NULL;

CREATE INDEX IF NOT EXISTS idx_states_pruned_at ON states(pruned_at);

-- Bump schema version
INSERT OR REPLACE INTO schema_version (version, applied_at)
    VALUES (9, strftime('%s', 'now'));


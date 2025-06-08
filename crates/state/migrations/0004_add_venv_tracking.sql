-- Add Python virtual environment tracking

-- Add venv_path column to packages table
ALTER TABLE packages ADD COLUMN venv_path TEXT;

-- Create index for packages with venvs
CREATE INDEX idx_packages_venv ON packages(state_id, name, version) WHERE venv_path IS NOT NULL;

-- Update schema version
UPDATE schema_version SET version = 4, applied_at = strftime('%s', 'now');
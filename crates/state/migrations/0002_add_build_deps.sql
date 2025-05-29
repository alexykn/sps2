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
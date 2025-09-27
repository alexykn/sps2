PRAGMA foreign_keys = ON;

-- Core state timeline ------------------------------------------------------
CREATE TABLE states (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES states(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    operation TEXT NOT NULL,
    success INTEGER NOT NULL DEFAULT 1,
    rollback_of TEXT REFERENCES states(id) ON DELETE SET NULL,
    pruned_at INTEGER
);
CREATE INDEX idx_states_created_at ON states(created_at DESC);
CREATE INDEX idx_states_parent_id ON states(parent_id);
CREATE INDEX idx_states_pruned_at ON states(pruned_at);

CREATE TABLE state_transitions (
    state_id TEXT PRIMARY KEY REFERENCES states(id) ON DELETE CASCADE,
    prev_state_id TEXT REFERENCES states(id) ON DELETE SET NULL,
    journal_phase TEXT NOT NULL CHECK (journal_phase IN ('Prepared', 'Swapped', 'Finalized')),
    staging_path TEXT NOT NULL,
    committed_at INTEGER,
    guard_digest TEXT
);
CREATE INDEX idx_state_transitions_phase ON state_transitions(journal_phase);
CREATE INDEX idx_state_transitions_committed_at ON state_transitions(committed_at DESC);

CREATE TABLE active_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    state_id TEXT NOT NULL REFERENCES states(id),
    updated_at INTEGER NOT NULL
);

-- Package metadata ---------------------------------------------------------
CREATE TABLE package_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    store_hash TEXT NOT NULL,
    package_hash TEXT,
    size_bytes INTEGER NOT NULL,
    manifest_json TEXT,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    UNIQUE(name, version)
);
CREATE INDEX idx_package_versions_name ON package_versions(name);
CREATE INDEX idx_package_versions_store_hash ON package_versions(store_hash);

CREATE TABLE package_deps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_version_id INTEGER NOT NULL REFERENCES package_versions(id) ON DELETE CASCADE,
    dep_name TEXT NOT NULL,
    dep_spec TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('runtime', 'build'))
);
CREATE INDEX idx_package_deps_pkg ON package_deps(package_version_id);
CREATE INDEX idx_package_deps_dep ON package_deps(dep_name);

CREATE TABLE state_packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    state_id TEXT NOT NULL REFERENCES states(id) ON DELETE CASCADE,
    package_version_id INTEGER NOT NULL REFERENCES package_versions(id) ON DELETE CASCADE,
    install_size_bytes INTEGER NOT NULL,
    added_at INTEGER NOT NULL,
    UNIQUE(state_id, package_version_id)
);
CREATE INDEX idx_state_packages_state ON state_packages(state_id);
CREATE INDEX idx_state_packages_package ON state_packages(package_version_id);
CREATE INDEX idx_state_packages_state_pkg ON state_packages(state_id, package_version_id);

-- Content-addressable store -------------------------------------------------
CREATE TABLE cas_objects (
    hash TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('archive', 'file')),
    size_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    ref_count INTEGER NOT NULL DEFAULT 0,
    is_executable INTEGER NOT NULL DEFAULT 0,
    is_symlink INTEGER NOT NULL DEFAULT 0,
    symlink_target TEXT,
    last_seen_at INTEGER,
    last_state_id TEXT REFERENCES states(id) ON DELETE SET NULL,
    last_removed_at INTEGER,
    CHECK ((is_symlink = 1 AND symlink_target IS NOT NULL) OR (is_symlink = 0 AND symlink_target IS NULL))
);
CREATE INDEX idx_cas_objects_kind ON cas_objects(kind);
CREATE INDEX idx_cas_objects_refcount ON cas_objects(ref_count);
CREATE INDEX idx_cas_objects_last_seen ON cas_objects(last_seen_at);

CREATE TABLE package_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_version_id INTEGER NOT NULL REFERENCES package_versions(id) ON DELETE CASCADE,
    file_hash TEXT NOT NULL REFERENCES cas_objects(hash),
    rel_path TEXT NOT NULL,
    mode INTEGER NOT NULL,
    uid INTEGER NOT NULL DEFAULT 0,
    gid INTEGER NOT NULL DEFAULT 0,
    mtime INTEGER,
    UNIQUE(package_version_id, rel_path)
);
CREATE INDEX idx_package_files_pkg ON package_files(package_version_id);
CREATE INDEX idx_package_files_hash ON package_files(file_hash);
CREATE INDEX idx_package_files_rel_path ON package_files(rel_path);
CREATE INDEX idx_package_files_pkg_hash ON package_files(package_version_id, file_hash);

CREATE TABLE file_verification (
    file_hash TEXT PRIMARY KEY REFERENCES cas_objects(hash) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'verified', 'failed', 'quarantined')),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_checked_at INTEGER,
    last_error TEXT
);
CREATE INDEX idx_file_verification_status ON file_verification(status, last_checked_at);

CREATE TABLE file_mtime_tracker (
    file_path TEXT PRIMARY KEY,
    last_verified_mtime INTEGER NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);
CREATE INDEX idx_file_mtime_tracker_mtime ON file_mtime_tracker(last_verified_mtime);

CREATE TRIGGER trg_file_mtime_tracker_utime
AFTER UPDATE ON file_mtime_tracker
BEGIN
    UPDATE file_mtime_tracker
    SET updated_at = strftime('%s', 'now')
    WHERE file_path = NEW.file_path;
END;

-- Operational logging -------------------------------------------------------
CREATE TABLE gc_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_at INTEGER NOT NULL,
    scope TEXT NOT NULL,
    items_removed INTEGER NOT NULL,
    bytes_freed INTEGER NOT NULL,
    notes TEXT
);
CREATE INDEX idx_gc_runs_scope ON gc_runs(scope, run_at DESC);

CREATE TABLE cas_evictions (
    hash TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('archive', 'file')),
    evicted_at INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL,
    reason TEXT
);
CREATE INDEX idx_cas_evictions_time ON cas_evictions(evicted_at);

-- Builder support -----------------------------------------------------------
CREATE TABLE build_envs (
    id TEXT PRIMARY KEY,
    package_version_id INTEGER REFERENCES package_versions(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    destroyed_at INTEGER,
    attrs TEXT
);
CREATE INDEX idx_build_envs_package ON build_envs(package_version_id);

CREATE TABLE build_env_deps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    env_id TEXT NOT NULL REFERENCES build_envs(id) ON DELETE CASCADE,
    dep_package_version_id INTEGER NOT NULL REFERENCES package_versions(id),
    UNIQUE(env_id, dep_package_version_id)
);
CREATE INDEX idx_build_env_deps_env ON build_env_deps(env_id);

PRAGMA user_version = 1;

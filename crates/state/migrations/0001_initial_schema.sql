-- Initial schema for spsv2 state management

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
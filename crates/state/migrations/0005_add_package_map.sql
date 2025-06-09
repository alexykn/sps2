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
# File-Level Content Addressable Storage - Database Schema Design

## Overview

This document outlines the database schema changes required to implement file-level content addressable storage in SPS2. The design extends the current package-level system to track individual files with their hashes, enabling granular verification, healing, and deduplication.

## Design Goals

1. **File-Level Tracking**: Store hash and metadata for every file in every package
2. **Efficient Deduplication**: Share identical files across packages
3. **Fast Verification**: Enable quick file integrity checks without store access
4. **Atomic Operations**: Maintain transactional integrity for file operations
5. **Performance**: Optimize for common query patterns
6. **Migration Safety**: Ensure smooth transition from current schema

## New Table Structures

### 1. `file_objects` - Content-Addressed File Storage

This table represents unique file content in the system, indexed by hash.

```sql
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
```

### 2. `package_file_entries` - Files Within Packages

This table maps files to their locations within packages, supporting deduplication.

```sql
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
```

### 3. `installed_files` - Tracking Installed File Locations

This table tracks where files are installed on the filesystem, replacing the current `package_files` table with hash-based tracking.

```sql
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
```

### 4. `file_verification_cache` - Optimization for Verification

This table caches recent file verification results to speed up repeated checks.

```sql
CREATE TABLE file_verification_cache (
    file_hash TEXT NOT NULL,            -- References file_objects(hash)
    installed_path TEXT NOT NULL,       -- Path that was verified
    verified_at INTEGER NOT NULL,       -- Unix timestamp of verification
    is_valid BOOLEAN NOT NULL,          -- Verification result
    error_message TEXT,                 -- Error details if verification failed
    PRIMARY KEY (file_hash, installed_path),
    FOREIGN KEY (file_hash) REFERENCES file_objects(hash) ON DELETE CASCADE
);

CREATE INDEX idx_file_verification_cache_verified_at ON file_verification_cache(verified_at);
```

## Modified Tables

### Updated `packages` Table

Add a computed package hash field for backward compatibility:

```sql
ALTER TABLE packages ADD COLUMN computed_hash TEXT;
ALTER TABLE packages ADD COLUMN has_file_hashes BOOLEAN NOT NULL DEFAULT 0;

-- Index for packages with file-level hashes
CREATE INDEX idx_packages_has_file_hashes ON packages(has_file_hashes) WHERE has_file_hashes = 1;
```

## Relationships and Constraints

### Entity Relationship Diagram

```
states (1) ----< (N) packages (1) ----< (N) package_file_entries
                        |                           |
                        |                           v
                        |                    file_objects (N) >---- (N) package_file_entries
                        |                           ^
                        |                           |
                        +----< (N) installed_files -+
```

### Key Relationships

1. **Package → Files**: One package contains many file entries
2. **File Object → Packages**: One file object can be referenced by multiple packages (deduplication)
3. **State → Installed Files**: Each state tracks which files are installed where
4. **File Hash Integrity**: All file references must point to valid file objects

## Indexing Strategy

### Primary Access Patterns

1. **Verification Queries**:
   - Find all files for a package: `package_id → file_hash, relative_path`
   - Verify installed file: `installed_path → file_hash`
   - Check file integrity: `file_hash → size, permissions`

2. **Installation Queries**:
   - Check if file exists: `hash → file_objects`
   - Find installation conflicts: `state_id, installed_path → existing files`
   - Track package files: `package_id → all files`

3. **Garbage Collection**:
   - Find unreferenced files: `ref_count = 0`
   - Find orphaned installations: `state_id not in active states`

### Index Design Rationale

- **Covering indexes** for common queries to avoid table lookups
- **Partial indexes** for garbage collection efficiency
- **Unique constraints** to ensure data integrity
- **Foreign key indexes** for join performance

## Migration Strategy

### Phase 1: Schema Addition (Non-Breaking)

1. Add new tables without removing old ones
2. Add `has_file_hashes` flag to packages table
3. Maintain backward compatibility

```sql
-- Migration 0006_add_file_level_hashes.sql
-- Add all new tables as shown above
-- No changes to existing data
```

### Phase 2: Data Migration (Background)

1. Process existing packages to compute file hashes
2. Populate new tables while maintaining old ones
3. Mark packages with `has_file_hashes = true` when processed

### Phase 3: Cutover (Atomic)

1. Switch code to use new tables
2. Deprecate old `package_files` table
3. Remove package-level hash dependency

### Phase 4: Cleanup (Safe)

1. Drop deprecated tables after verification
2. Remove backward compatibility code

## Performance Considerations

### Query Performance Estimates

| Query Type | Estimated Time | Index Used |
|------------|---------------|------------|
| Find package files | < 1ms | idx_package_file_entries_package_id |
| Verify file hash | < 1ms | PRIMARY (file_objects) |
| Find file installations | < 1ms | idx_installed_files_file_hash |
| Check path conflicts | < 1ms | UNIQUE(state_id, installed_path) |
| GC unreferenced files | < 10ms | idx_file_objects_ref_count |

### Storage Overhead

- **File object record**: ~100 bytes per unique file
- **Package file entry**: ~150 bytes per file per package
- **Installed file record**: ~200 bytes per installed file
- **Estimated growth**: 10-20% increase in database size

### Optimization Strategies

1. **Batch insertions** during package installation
2. **Prepared statements** for repeated queries
3. **Connection pooling** for concurrent operations
4. **Vacuum scheduling** for space reclamation
5. **Cache warming** for frequently accessed files

## Security Considerations

1. **Path Normalization**: Prevent path traversal attacks
2. **Hash Validation**: Ensure hash format consistency
3. **Permission Preservation**: Maintain correct file permissions
4. **Symlink Handling**: Validate symlink targets

## Future Extensions

1. **Compression Tracking**: Store compression method per file
2. **Signature Storage**: Add cryptographic signatures
3. **Metadata Extensions**: Support extended attributes
4. **Delta Storage**: Store file deltas for updates

## Validation Queries

### Data Integrity Checks

```sql
-- Check for orphaned file entries
SELECT COUNT(*) FROM package_file_entries pfe
LEFT JOIN file_objects fo ON pfe.file_hash = fo.hash
WHERE fo.hash IS NULL;

-- Check for unreferenced file objects
SELECT COUNT(*) FROM file_objects
WHERE ref_count = 0 AND hash NOT IN (
    SELECT DISTINCT file_hash FROM package_file_entries
);

-- Verify installation consistency
SELECT COUNT(*) FROM installed_files if
LEFT JOIN file_objects fo ON if.file_hash = fo.hash
WHERE fo.hash IS NULL;
```

### Performance Validation

```sql
-- Measure file lookup performance
EXPLAIN QUERY PLAN
SELECT * FROM package_file_entries
WHERE package_id = ? AND relative_path = ?;

-- Measure deduplication effectiveness
SELECT 
    COUNT(DISTINCT file_hash) as unique_files,
    COUNT(*) as total_references,
    CAST(COUNT(*) AS REAL) / COUNT(DISTINCT file_hash) as dedup_ratio
FROM package_file_entries;
```

## Implementation Notes

1. **Hash Format**: Store as lowercase hex string (64 characters)
2. **Path Normalization**: Remove leading slashes, collapse `..` and `.`
3. **Transaction Boundaries**: Wrap file operations in transactions
4. **Error Handling**: Graceful degradation for missing files
5. **Logging**: Detailed logging for migration and verification

This schema design provides a solid foundation for implementing file-level content addressable storage while maintaining backward compatibility and ensuring optimal performance.
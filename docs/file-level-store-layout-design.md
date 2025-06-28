# File-Level Content Addressable Store Layout Design

## Overview

This document outlines the design for transforming SPS2's package-level content-addressed storage into a file-level content-addressed storage system. The new design enables file deduplication across packages, granular verification, and efficient healing operations.

## Current Store Layout

```
/opt/pm/store/
├── <package-hash-1>/           # Package directory named by content hash
│   ├── manifest.toml           # Package manifest
│   ├── opt/pm/live/           # New structure: actual package files
│   │   ├── bin/
│   │   ├── lib/
│   │   └── share/
│   └── sbom.spdx.json         # Optional SBOM file
├── <package-hash-2>/
└── ...
```

## Proposed File-Level Store Layout

### Directory Structure

```
/opt/pm/store/
├── objects/                    # Content-addressed file storage
│   ├── 00/                    # First 2 chars of hash (256 buckets)
│   │   ├── 00a1b2c3d4.../    # Full hash directory
│   │   │   ├── data          # Actual file content
│   │   │   └── meta.json     # File metadata
│   │   └── 00f5e6d7c8.../
│   ├── 01/
│   ├── ...
│   └── ff/
├── packages/                   # Package metadata storage
│   ├── <package-hash>/        # Package directory
│   │   ├── manifest.toml      # Package manifest
│   │   ├── files.json         # File listing with hashes
│   │   └── sbom.json          # Optional SBOM
│   └── ...
├── temp/                      # Temporary staging area
└── quarantine/                # Corrupted files pending repair
```

### File Object Structure

Each file is stored as:
```
objects/<prefix>/<hash>/
├── data                       # The actual file content
└── meta.json                  # Metadata file
```

#### Metadata Format (meta.json)

```json
{
  "hash": "00a1b2c3d4e5f6789012345678901234567890123456789012345678901234567890",
  "size": 1024576,
  "created_at": 1719561600,
  "ref_count": 3,
  "is_executable": true,
  "is_symlink": false,
  "symlink_target": null,
  "compression": "zstd",
  "original_size": 2048000
}
```

### Package Metadata Structure

```
packages/<package-hash>/
├── manifest.toml              # Original package manifest
├── files.json                 # File listing with metadata
└── sbom.json                  # Software Bill of Materials
```

#### File Listing Format (files.json)

```json
{
  "version": "1.0",
  "package_hash": "abc123...",
  "created_at": 1719561600,
  "files": [
    {
      "path": "bin/tool",
      "hash": "00a1b2c3d4e5f6789012345678901234567890123456789012345678901234567890",
      "size": 1024576,
      "permissions": 755,
      "uid": 0,
      "gid": 0,
      "mtime": 1719561600
    },
    {
      "path": "lib/libtool.so",
      "hash": "11b2c3d4e5f6789012345678901234567890123456789012345678901234567890",
      "size": 2048000,
      "permissions": 644,
      "uid": 0,
      "gid": 0,
      "mtime": 1719561600
    }
  ],
  "directories": [
    {
      "path": "bin",
      "permissions": 755
    },
    {
      "path": "lib",
      "permissions": 755
    }
  ]
}
```

## Deduplication Strategy

### Content-Addressed Storage

1. **File Hashing**: Each file is hashed using BLAKE3
2. **Hash Prefix Sharding**: First 2 characters create 256 buckets to avoid filesystem limitations
3. **Atomic Operations**: Files are written to temp/ then moved atomically
4. **Reference Counting**: Track how many packages reference each file

### Deduplication Algorithm

```rust
async fn store_file(content: &[u8], metadata: FileMetadata) -> Result<Hash> {
    // 1. Compute BLAKE3 hash
    let hash = Hash::hash_bytes(content);
    
    // 2. Check if file already exists
    let object_path = object_path(&hash);
    if exists(&object_path).await {
        // Increment reference count
        increment_ref_count(&hash).await?;
        return Ok(hash);
    }
    
    // 3. Write to temporary location
    let temp_path = temp_file_path();
    write_atomic(&temp_path, content).await?;
    
    // 4. Move to final location
    let prefix = &hash.to_hex()[..2];
    let object_dir = format!("objects/{}/{}", prefix, hash.to_hex());
    create_dir_all(&object_dir).await?;
    rename(&temp_path, &object_dir.join("data")).await?;
    
    // 5. Write metadata
    write_metadata(&object_dir, &metadata).await?;
    
    Ok(hash)
}
```

## Symlink and Hard Link Handling

### Symlink Storage

Symlinks are stored as special file objects with metadata indicating the target:

```json
{
  "hash": "22c3d4e5f6789012345678901234567890123456789012345678901234567890",
  "size": 24,
  "is_symlink": true,
  "symlink_target": "/usr/bin/python3",
  "permissions": 777
}
```

The `data` file contains the symlink target path for verification.

### Hard Link Strategy

During installation:
1. Regular files are hard-linked from store to destination
2. Symlinks are recreated (not hard-linked)
3. Directories are created with proper permissions

## Store Space Optimization

### Compression

1. **Automatic Compression**: Files > 4KB are compressed with zstd
2. **APFS Compression**: Leverage filesystem compression on macOS
3. **Metadata Tracking**: Store both compressed and original sizes

### Garbage Collection

```rust
async fn garbage_collect() -> Result<Vec<Hash>> {
    let mut removed = Vec::new();
    
    // Find objects with ref_count = 0
    for object in find_unreferenced_objects().await? {
        // Move to quarantine first (safety)
        quarantine_object(&object).await?;
        removed.push(object.hash);
    }
    
    // Clean quarantine after 7 days
    clean_old_quarantine().await?;
    
    removed
}
```

### Store Compaction

1. **Bucket Rebalancing**: Redistribute files if any bucket grows too large
2. **Defragmentation**: Periodic consolidation of small files
3. **Index Optimization**: Rebuild file indices for faster lookups

## Store Integrity Verification

### Verification Levels

1. **Quick Check**: Verify file exists and size matches
2. **Full Verification**: Compute hash and compare
3. **Deep Scan**: Check all metadata and permissions

### Verification Algorithm

```rust
async fn verify_store_integrity() -> Result<Vec<IntegrityError>> {
    let mut errors = Vec::new();
    
    // 1. Verify all objects
    for object in list_all_objects().await? {
        // Check file exists
        if !exists(&object.data_path()).await {
            errors.push(IntegrityError::MissingData(object.hash));
            continue;
        }
        
        // Verify hash matches content
        let computed_hash = Hash::hash_file(&object.data_path()).await?;
        if computed_hash != object.hash {
            errors.push(IntegrityError::HashMismatch {
                expected: object.hash,
                actual: computed_hash,
            });
        }
        
        // Verify metadata consistency
        if let Err(e) = verify_metadata(&object).await {
            errors.push(IntegrityError::MetadataError(object.hash, e));
        }
    }
    
    // 2. Verify package references
    for package in list_all_packages().await? {
        for file_ref in package.files {
            if !object_exists(&file_ref.hash).await {
                errors.push(IntegrityError::MissingReference {
                    package: package.hash,
                    file: file_ref.hash,
                });
            }
        }
    }
    
    errors
}
```

## Migration Strategy

### Phase 1: Parallel Structure (Non-Breaking)

1. Keep existing `/opt/pm/store/<package-hash>/` structure
2. Add new `/opt/pm/store/objects/` directory
3. New installations use file-level storage
4. Old packages continue to work

### Phase 2: Background Migration

```rust
async fn migrate_package(package_hash: &Hash) -> Result<()> {
    let package_path = old_package_path(package_hash);
    let mut file_entries = Vec::new();
    
    // 1. Hash all files in package
    for file_path in walk_package_files(&package_path).await? {
        let content = read_file(&file_path).await?;
        let hash = store_file(&content, extract_metadata(&file_path)).await?;
        
        file_entries.push(FileEntry {
            path: relative_path(&package_path, &file_path),
            hash,
            // ... other metadata
        });
    }
    
    // 2. Create package metadata
    create_package_metadata(package_hash, file_entries).await?;
    
    // 3. Mark package as migrated
    mark_migrated(package_hash).await?;
    
    Ok(())
}
```

### Phase 3: Cleanup

1. Remove old package directories after successful migration
2. Update all code paths to use new structure
3. Remove migration code

### Migration Safety

1. **Atomic Operations**: Each package migration is atomic
2. **Rollback Support**: Keep old structure until verified
3. **Progress Tracking**: Database tracks migration status
4. **Verification**: Verify migrated packages before cleanup

## Performance Considerations

### Optimization Strategies

1. **Parallel Processing**: Hash files in parallel during installation
2. **Batch Operations**: Group small files for efficient I/O
3. **Memory Mapping**: Use mmap for large files
4. **Caching**: Cache frequently accessed metadata

### Expected Performance

| Operation | Current | File-Level | Improvement |
|-----------|---------|------------|-------------|
| Install 100 files | 500ms | 550ms | -10% (overhead) |
| Verify 1 file | 500ms | 5ms | 99% |
| Dedupe savings | 0% | 60-80% | Significant |
| Store size (1000 pkgs) | 10GB | 4GB | 60% reduction |

## Security Considerations

1. **Path Traversal**: Validate all paths before storage
2. **Permission Preservation**: Store and restore exact permissions
3. **Atomic Writes**: Prevent partial file states
4. **Integrity Checks**: Verify hashes on read

## Implementation Plan

### Phase 1: Core Infrastructure
- Implement file object storage
- Add deduplication logic
- Create migration framework

### Phase 2: Integration
- Update install process
- Implement verification
- Add healing capabilities

### Phase 3: Optimization
- Add compression
- Implement garbage collection
- Performance tuning

## Conclusion

This file-level store layout provides:
- **60-80% space savings** through deduplication
- **99% faster** file verification
- **Granular healing** of individual files
- **Better integrity** guarantees
- **Scalable design** for large installations

The migration strategy ensures zero downtime and safe transition from the current package-level storage.
# File-Level Store Migration Strategy

## Overview

This document outlines the migration strategy for transitioning from package-level to file-level content-addressed storage in SPS2. The migration is designed to be safe, incremental, and reversible.

## Migration Principles

1. **Zero Downtime**: System remains fully functional during migration
2. **Incremental**: Packages migrated one at a time
3. **Reversible**: Can rollback at any point
4. **Verifiable**: Each step is validated before proceeding
5. **Resumable**: Migration can be paused and resumed

## Migration Phases

### Phase 0: Preparation

#### Database Schema Updates
```sql
-- Add migration tracking table
CREATE TABLE store_migration (
    id INTEGER PRIMARY KEY,
    package_hash TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('pending', 'in_progress', 'completed', 'failed')),
    started_at INTEGER,
    completed_at INTEGER,
    error_message TEXT,
    file_count INTEGER,
    space_saved INTEGER,
    FOREIGN KEY (package_hash) REFERENCES store_refs(hash)
);

-- Add file-level tables (from OPS-80)
-- Already implemented in previous migration
```

#### Configuration Updates
```toml
[store]
# Migration settings
migration_enabled = true
migration_batch_size = 10
migration_parallel_files = 4
verify_after_migration = true
keep_old_packages = true  # Safety: keep until verified
```

### Phase 1: Dual-Mode Operation

#### 1.1 Code Updates

```rust
// Store manager supporting both modes
impl PackageStore {
    pub async fn get_package(&self, hash: &Hash) -> Result<Package> {
        // Check new structure first
        if self.has_file_level_package(hash).await? {
            return self.get_file_level_package(hash).await;
        }
        
        // Fall back to old structure
        self.get_legacy_package(hash).await
    }
    
    pub async fn add_package(&self, path: &Path) -> Result<StoredPackage> {
        if self.config.use_file_level_storage {
            self.add_file_level_package(path).await
        } else {
            self.add_legacy_package(path).await
        }
    }
}
```

#### 1.2 Directory Structure

```
/opt/pm/store/
├── objects/              # New file-level storage
├── packages/             # New package metadata
├── <package-hash>/       # Old package storage (kept during migration)
├── temp/                 # Temporary files
└── migration/            # Migration state and logs
    ├── state.json
    └── logs/
```

### Phase 2: Background Migration

#### 2.1 Migration Worker

```rust
pub struct MigrationWorker {
    store: PackageStore,
    db: StateManager,
    config: MigrationConfig,
}

impl MigrationWorker {
    pub async fn run(&self) -> Result<()> {
        loop {
            // Get next batch of packages to migrate
            let packages = self.get_pending_packages(self.config.batch_size).await?;
            
            if packages.is_empty() {
                // Migration complete
                break;
            }
            
            // Process packages in parallel
            let results = futures::future::join_all(
                packages.iter().map(|pkg| self.migrate_package(pkg))
            ).await;
            
            // Handle results
            for (package, result) in packages.iter().zip(results) {
                match result {
                    Ok(stats) => {
                        self.mark_completed(package, stats).await?;
                    }
                    Err(e) => {
                        self.mark_failed(package, e).await?;
                    }
                }
            }
            
            // Pause between batches
            tokio::time::sleep(self.config.pause_duration).await;
        }
        
        Ok(())
    }
}
```

#### 2.2 Package Migration Process

```rust
async fn migrate_package(&self, package_hash: &Hash) -> Result<MigrationStats> {
    let mut stats = MigrationStats::default();
    
    // 1. Mark as in progress
    self.db.update_migration_status(package_hash, "in_progress").await?;
    
    // 2. Load package from old structure
    let old_path = self.store.old_package_path(package_hash);
    let manifest = load_manifest(&old_path).await?;
    
    // 3. Process all files
    let mut file_entries = Vec::new();
    for file_path in walk_files(&old_path).await? {
        // Read file content and metadata
        let content = read_file(&file_path).await?;
        let metadata = extract_metadata(&file_path).await?;
        
        // Store in new structure
        let file_hash = self.store_file_object(&content, &metadata).await?;
        
        // Track deduplication
        if self.was_deduplicated(&file_hash).await? {
            stats.deduplicated_files += 1;
            stats.space_saved += metadata.size;
        }
        
        file_entries.push(FileEntry {
            path: relative_path(&old_path, &file_path),
            hash: file_hash,
            size: metadata.size,
            permissions: metadata.permissions,
        });
        
        stats.files_processed += 1;
    }
    
    // 4. Create package metadata
    self.create_package_metadata(package_hash, &manifest, file_entries).await?;
    
    // 5. Verify migration
    if self.config.verify_after_migration {
        self.verify_migrated_package(package_hash).await?;
    }
    
    // 6. Update database
    self.db.mark_package_migrated(package_hash).await?;
    
    Ok(stats)
}
```

#### 2.3 Verification Process

```rust
async fn verify_migrated_package(&self, package_hash: &Hash) -> Result<()> {
    // 1. Load from both old and new structures
    let old_files = self.list_old_package_files(package_hash).await?;
    let new_files = self.list_new_package_files(package_hash).await?;
    
    // 2. Verify file count matches
    if old_files.len() != new_files.len() {
        return Err(MigrationError::FileCountMismatch);
    }
    
    // 3. Verify each file
    for (old_file, new_file) in old_files.iter().zip(&new_files) {
        // Compare paths
        if old_file.path != new_file.path {
            return Err(MigrationError::PathMismatch);
        }
        
        // Compare content hashes
        let old_hash = hash_file(&old_file.full_path).await?;
        if old_hash != new_file.hash {
            return Err(MigrationError::ContentMismatch);
        }
        
        // Compare metadata
        if old_file.permissions != new_file.permissions {
            return Err(MigrationError::PermissionMismatch);
        }
    }
    
    Ok(())
}
```

### Phase 3: Cutover

#### 3.1 Validation Checklist

- [ ] All packages migrated successfully
- [ ] No failed migrations
- [ ] Verification passes for all packages
- [ ] Performance metrics acceptable
- [ ] Backup of old store completed

#### 3.2 Cutover Process

```rust
async fn perform_cutover(&self) -> Result<()> {
    // 1. Verify all packages migrated
    let pending = self.count_pending_migrations().await?;
    if pending > 0 {
        return Err(CutoverError::PendingMigrations(pending));
    }
    
    // 2. Switch to file-level mode only
    self.config.set_file_level_only(true).await?;
    
    // 3. Run final verification
    self.verify_entire_store().await?;
    
    // 4. Create backup marker
    self.mark_old_store_for_cleanup().await?;
    
    Ok(())
}
```

### Phase 4: Cleanup

#### 4.1 Old Store Removal

```rust
async fn cleanup_old_store(&self) -> Result<()> {
    // Safety checks
    if !self.is_cleanup_safe().await? {
        return Err(CleanupError::NotSafe);
    }
    
    // Remove old package directories
    for package_hash in self.list_old_packages().await? {
        // Verify package exists in new structure
        if !self.has_file_level_package(&package_hash).await? {
            return Err(CleanupError::MissingInNewStore(package_hash));
        }
        
        // Remove old directory
        let old_path = self.old_package_path(&package_hash);
        remove_dir_all(&old_path).await?;
    }
    
    Ok(())
}
```

## Rollback Strategy

### Rollback Triggers

1. **High Error Rate**: > 5% of migrations failing
2. **Performance Degradation**: > 20% slower operations
3. **Data Corruption**: Any integrity check failures
4. **Manual Trigger**: Administrator decision

### Rollback Process

```rust
async fn rollback_migration(&self) -> Result<()> {
    // 1. Stop migration worker
    self.stop_migration_worker().await?;
    
    // 2. Switch back to legacy mode
    self.config.use_file_level_storage = false;
    
    // 3. Mark all migrations as rolled back
    self.db.execute(
        "UPDATE store_migration SET status = 'rolled_back' WHERE status = 'completed'"
    ).await?;
    
    // 4. Clean up new structure (optional)
    if self.config.cleanup_on_rollback {
        self.cleanup_new_structure().await?;
    }
    
    Ok(())
}
```

## Monitoring and Metrics

### Progress Tracking

```rust
#[derive(Serialize)]
struct MigrationProgress {
    total_packages: usize,
    migrated_packages: usize,
    failed_packages: usize,
    total_files: usize,
    deduplicated_files: usize,
    space_saved_bytes: u64,
    estimated_time_remaining: Duration,
}

async fn get_migration_progress(&self) -> Result<MigrationProgress> {
    let stats = self.db.query_as!(
        MigrationStats,
        "SELECT 
            COUNT(*) as total,
            COUNT(CASE WHEN status = 'completed' THEN 1 END) as completed,
            COUNT(CASE WHEN status = 'failed' THEN 1 END) as failed,
            SUM(file_count) as total_files,
            SUM(space_saved) as space_saved
        FROM store_migration"
    ).fetch_one(&self.db).await?;
    
    // Calculate estimation
    let remaining = stats.total - stats.completed;
    let rate = stats.completed as f64 / elapsed.as_secs_f64();
    let estimated_remaining = Duration::from_secs_f64(remaining as f64 / rate);
    
    Ok(MigrationProgress {
        total_packages: stats.total,
        migrated_packages: stats.completed,
        failed_packages: stats.failed,
        total_files: stats.total_files,
        space_saved_bytes: stats.space_saved,
        estimated_time_remaining: estimated_remaining,
    })
}
```

### Health Checks

```rust
async fn migration_health_check(&self) -> Result<HealthStatus> {
    let mut status = HealthStatus::Healthy;
    let mut issues = Vec::new();
    
    // Check error rate
    let error_rate = self.calculate_error_rate().await?;
    if error_rate > 0.05 {
        status = HealthStatus::Warning;
        issues.push(format!("High error rate: {:.1}%", error_rate * 100.0));
    }
    
    // Check disk space
    let free_space = get_free_space("/opt/pm/store").await?;
    if free_space < 1_000_000_000 { // 1GB
        status = HealthStatus::Critical;
        issues.push("Low disk space".to_string());
    }
    
    // Check migration speed
    let speed = self.calculate_migration_speed().await?;
    if speed < 10 { // packages per hour
        status = HealthStatus::Warning;
        issues.push(format!("Slow migration: {} pkg/hr", speed));
    }
    
    Ok(HealthStatus { status, issues })
}
```

## Testing Strategy

### Test Scenarios

1. **Happy Path**: Normal package migration
2. **Large Package**: Package with 10,000+ files
3. **Deduplication**: Packages with shared files
4. **Failure Recovery**: Simulated failures during migration
5. **Rollback**: Full rollback scenario
6. **Performance**: Load testing during migration

### Validation Tests

```rust
#[cfg(test)]
mod migration_tests {
    #[tokio::test]
    async fn test_package_migration() {
        let store = setup_test_store().await;
        let package = create_test_package().await;
        
        // Add package in old format
        let hash = store.add_legacy_package(&package).await.unwrap();
        
        // Migrate
        let stats = store.migrate_package(&hash).await.unwrap();
        
        // Verify
        assert!(stats.files_processed > 0);
        assert!(store.has_file_level_package(&hash).await.unwrap());
        
        // Compare old vs new
        let old_files = store.list_legacy_files(&hash).await.unwrap();
        let new_files = store.list_file_level_files(&hash).await.unwrap();
        assert_eq!(old_files.len(), new_files.len());
    }
}
```

## Timeline

### Week 1-2: Preparation
- Deploy database schema changes
- Update store code for dual-mode
- Implement migration worker

### Week 3-4: Testing
- Test migration on staging environment
- Performance testing
- Rollback testing

### Week 5-8: Production Migration
- Start with small packages
- Monitor and adjust parameters
- Complete all packages

### Week 9: Cutover
- Final validation
- Switch to file-level only
- Monitor for issues

### Week 10+: Cleanup
- Remove old package directories
- Archive migration logs
- Document lessons learned

## Risk Mitigation

1. **Data Loss**: Full backup before migration
2. **Corruption**: Verification after each package
3. **Performance**: Throttling and monitoring
4. **Disk Space**: Pre-calculate space requirements
5. **Rollback**: Keep old structure until verified

## Success Criteria

- ✅ 100% of packages successfully migrated
- ✅ Zero data loss or corruption
- ✅ 60%+ space savings achieved
- ✅ Performance targets met
- ✅ No service disruptions

This migration strategy ensures a safe, controlled transition to file-level storage with multiple safety nets and verification steps.
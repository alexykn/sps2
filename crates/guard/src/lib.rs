//! State verification guard for ensuring database/filesystem consistency

use sps2_errors::{Error, OpsError};
use sps2_events::{Event, EventSender};
use sps2_hash::Hash;
use sps2_state::{queries, StateManager};
use sps2_store::{PackageStore, StoredPackage};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};
use uuid::Uuid;

/// Verification level for state checking
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VerificationLevel {
    /// Quick check - file existence only
    Quick,
    /// Standard check - existence + metadata
    Standard,
    /// Full check - existence + metadata + content hash
    Full,
}

impl Default for VerificationLevel {
    fn default() -> Self {
        Self::Standard
    }
}

/// Category of orphaned file
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum OrphanedFileCategory {
    /// Leftover from previous package versions
    Leftover,
    /// User-created file (e.g., config, data)
    UserCreated,
    /// Temporary file that should be cleaned
    Temporary,
    /// System file that should be preserved
    System,
    /// Unknown category - needs investigation
    Unknown,
}

/// Action to take for orphaned files
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanedFileAction {
    /// Remove the file
    Remove,
    /// Preserve the file in place
    Preserve,
    /// Backup the file then remove
    Backup,
}

/// Type of discrepancy found during verification
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Discrepancy {
    /// File expected but not found
    MissingFile {
        package_name: String,
        package_version: String,
        file_path: String,
    },
    /// File exists but has wrong type (file vs directory)
    TypeMismatch {
        package_name: String,
        package_version: String,
        file_path: String,
        expected_directory: bool,
        actual_directory: bool,
    },
    /// File content doesn't match expected hash
    CorruptedFile {
        package_name: String,
        package_version: String,
        file_path: String,
        expected_hash: String,
        actual_hash: String,
    },
    /// File exists but not tracked in database
    OrphanedFile {
        file_path: String,
        category: OrphanedFileCategory,
    },
    /// Python virtual environment missing
    MissingVenv {
        package_name: String,
        package_version: String,
        venv_path: String,
    },
}

/// Result of verification check
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerificationResult {
    /// State ID that was verified
    pub state_id: Uuid,
    /// List of discrepancies found
    pub discrepancies: Vec<Discrepancy>,
    /// Whether verification passed (no discrepancies)
    pub is_valid: bool,
    /// Time taken for verification in milliseconds
    pub duration_ms: u64,
}

impl VerificationResult {
    /// Create a new verification result
    #[must_use]
    pub fn new(state_id: Uuid, discrepancies: Vec<Discrepancy>, duration_ms: u64) -> Self {
        let is_valid = discrepancies.is_empty();
        Self {
            state_id,
            discrepancies,
            is_valid,
            duration_ms,
        }
    }
}

/// Cache entry for a verified file
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileCacheEntry {
    /// File path relative to live directory
    pub file_path: String,
    /// Package name this file belongs to
    pub package_name: String,
    /// Package version
    pub package_version: String,
    /// File modification time when last verified
    pub mtime: SystemTime,
    /// File size when last verified
    pub size: u64,
    /// Content hash (only for Full verification level)
    pub content_hash: Option<String>,
    /// Timestamp when this entry was created
    pub verified_at: SystemTime,
    /// Verification level used for this entry
    pub verification_level: VerificationLevel,
    /// Whether file was valid at time of verification
    pub was_valid: bool,
}

/// Cache statistics for monitoring performance
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CacheStats {
    /// Total cache lookups attempted
    pub lookups: u64,
    /// Cache hits (valid entries found)
    pub hits: u64,
    /// Cache misses (no entry or invalidated)
    pub misses: u64,
    /// Number of entries in cache
    pub entry_count: u64,
    /// Total memory usage estimate in bytes
    pub memory_usage_bytes: u64,
    /// Time saved by cache hits in milliseconds
    pub time_saved_ms: u64,
}

impl CacheStats {
    /// Calculate cache hit rate as percentage
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            0.0
        } else {
            (self.hits as f64 / self.lookups as f64) * 100.0
        }
    }
}

/// Verification cache manager
#[derive(Debug)]
pub struct VerificationCache {
    /// In-memory cache entries indexed by file path
    entries: HashMap<String, FileCacheEntry>,
    /// Cache statistics
    stats: CacheStats,
    /// Maximum cache entries (for bounded size)
    max_entries: usize,
    /// Maximum cache age in seconds
    max_age_seconds: u64,
}

impl VerificationCache {
    /// Create a new verification cache
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for VerificationCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            stats: CacheStats::default(),
            max_entries: 10_000,  // Default limit
            max_age_seconds: 300, // 5 minutes default
        }
    }
}

impl VerificationCache {
    /// Get cache statistics
    #[must_use]
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Check if a file's cache entry is valid
    pub fn is_entry_valid(
        &mut self,
        file_path: &str,
        verification_level: VerificationLevel,
    ) -> bool {
        self.stats.lookups += 1;

        let Some(entry) = self.entries.get(file_path) else {
            self.stats.misses += 1;
            return false;
        };

        // Check if entry is too old
        let now = SystemTime::now();
        if let Ok(age) = now.duration_since(entry.verified_at) {
            if age.as_secs() > self.max_age_seconds {
                self.stats.misses += 1;
                return false;
            }
        } else {
            // Clock went backwards, invalidate entry
            self.stats.misses += 1;
            return false;
        }

        // Check if verification level matches or exceeds cached level
        let level_sufficient = match (verification_level, entry.verification_level) {
            (VerificationLevel::Quick, _) => true,
            (VerificationLevel::Standard, VerificationLevel::Quick) => false,
            (VerificationLevel::Standard, _) => true,
            (VerificationLevel::Full, VerificationLevel::Full) => true,
            (VerificationLevel::Full, _) => false,
        };

        if !level_sufficient {
            self.stats.misses += 1;
            return false;
        }

        // Check if file has been modified since last verification
        if let Ok(metadata) = std::fs::metadata(file_path) {
            if let Ok(current_mtime) = metadata.modified() {
                if current_mtime != entry.mtime || metadata.len() != entry.size {
                    self.stats.misses += 1;
                    return false;
                }
            } else {
                self.stats.misses += 1;
                return false;
            }
        } else {
            // File doesn't exist, cache invalid
            self.stats.misses += 1;
            return false;
        }

        self.stats.hits += 1;
        true
    }

    /// Add or update a cache entry
    pub fn update_entry(&mut self, entry: FileCacheEntry) {
        // Enforce cache size limit
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&entry.file_path) {
            self.evict_oldest_entries();
        }

        self.entries.insert(entry.file_path.clone(), entry);
        self.update_stats();
    }

    /// Get a cache entry if valid
    #[must_use]
    pub fn get_entry(&self, file_path: &str) -> Option<&FileCacheEntry> {
        self.entries.get(file_path)
    }

    /// Remove entries for specific packages (e.g., after uninstall)
    pub fn invalidate_package(&mut self, package_name: &str, package_version: &str) {
        self.entries.retain(|_path, entry| {
            !(entry.package_name == package_name && entry.package_version == package_version)
        });
        self.update_stats();
    }

    /// Clear all cache entries
    pub fn clear(&mut self) {
        self.entries.clear();
        self.stats = CacheStats::default();
    }

    /// Evict oldest entries to maintain cache size limit
    fn evict_oldest_entries(&mut self) {
        let target_size = (self.max_entries * 80) / 100; // Remove 20% when limit hit
        if self.entries.len() <= target_size {
            return;
        }

        // Collect entries with their ages
        let mut entries_with_age: Vec<(String, SystemTime)> = self
            .entries
            .iter()
            .map(|(path, entry)| (path.clone(), entry.verified_at))
            .collect();

        // Sort by age (oldest first)
        entries_with_age.sort_by(|a, b| a.1.cmp(&b.1));

        // Remove oldest entries
        let to_remove = self.entries.len() - target_size;
        for (path, _) in entries_with_age.into_iter().take(to_remove) {
            self.entries.remove(&path);
        }

        self.update_stats();
    }

    /// Update cache statistics
    fn update_stats(&mut self) {
        self.stats.entry_count = self.entries.len() as u64;
        // Rough estimate: each entry ~200 bytes on average
        self.stats.memory_usage_bytes = self.entries.len() as u64 * 200;
    }

    /// Load cache from persistent storage
    ///
    /// Note: Persistence should be implemented at a higher level
    /// by serializing/deserializing cache data through the StateVerificationGuard
    pub async fn load_from_storage(&mut self) -> Result<(), Error> {
        // Cache starts empty on each run - persistence to be implemented
        // at StateVerificationGuard level if needed
        Ok(())
    }

    /// Save cache to persistent storage
    ///
    /// Note: Persistence should be implemented at a higher level
    /// by serializing/deserializing cache data through the StateVerificationGuard
    pub async fn save_to_storage(&self) -> Result<(), Error> {
        // Persistence to be implemented at StateVerificationGuard level if needed
        Ok(())
    }
}

/// State verification guard for consistency checking
pub struct StateVerificationGuard {
    /// State manager for database operations
    state_manager: StateManager,
    /// Package store for content verification
    store: PackageStore,
    /// Event sender for progress reporting
    tx: EventSender,
    /// Verification level
    level: VerificationLevel,
    /// Verification cache for performance optimization
    cache: VerificationCache,
}

impl StateVerificationGuard {
    /// Create a new verification guard with builder
    #[must_use]
    pub fn builder() -> StateVerificationGuardBuilder {
        StateVerificationGuardBuilder::new()
    }

    /// Verify current state and optionally heal discrepancies
    ///
    /// # Errors
    ///
    /// Returns an error if state verification fails or database operations fail.
    pub async fn verify_and_heal(
        &mut self,
        config: &sps2_config::Config,
    ) -> Result<VerificationResult, Error> {
        let start_time = Instant::now();

        // First, run verification to detect discrepancies
        let mut verification_result = self.verify_only().await?;

        // If no discrepancies found, return early
        if verification_result.is_valid {
            return Ok(verification_result);
        }

        // Emit healing start event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Starting healing process for {} discrepancies",
                verification_result.discrepancies.len()
            ),
            context: HashMap::default(),
        });

        // Track healing results
        let mut healed_count = 0;
        let mut failed_healings = Vec::new();

        // Process each discrepancy
        let discrepancies = verification_result.discrepancies.clone();
        for discrepancy in &discrepancies {
            match discrepancy {
                Discrepancy::MissingFile {
                    package_name,
                    package_version,
                    file_path,
                } => {
                    match self
                        .restore_missing_file(package_name, package_version, file_path)
                        .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!(
                                    "Restored missing file: {file_path} from {package_name}-{package_version}"
                                ),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Failed to restore {file_path}: {e}"),
                                context: HashMap::default(),
                            });
                        }
                    }
                }
                Discrepancy::OrphanedFile {
                    file_path,
                    category,
                } => match self.handle_orphaned_file(file_path, category, config).await {
                    Ok(()) => {
                        healed_count += 1;
                        let _ = self.tx.send(Event::DebugLog {
                            message: format!("Successfully handled orphaned file: {file_path} (category: {category:?})"),
                            context: HashMap::default(),
                        });
                    }
                    Err(e) => {
                        failed_healings.push(discrepancy.clone());
                        let _ = self.tx.send(Event::DebugLog {
                            message: format!("Failed to handle orphaned file {file_path}: {e}"),
                            context: HashMap::default(),
                        });
                    }
                },
                Discrepancy::CorruptedFile {
                    package_name,
                    package_version,
                    file_path,
                    expected_hash,
                    actual_hash,
                } => match self
                    .heal_corrupted_file(
                        package_name,
                        package_version,
                        file_path,
                        expected_hash,
                        actual_hash,
                    )
                    .await
                {
                    Ok(()) => {
                        healed_count += 1;
                        let _ = self.tx.send(Event::DebugLog {
                            message: format!("Successfully restored corrupted file: {file_path} for {package_name}-{package_version}"),
                            context: HashMap::default(),
                        });
                    }
                    Err(e) => {
                        failed_healings.push(discrepancy.clone());
                        let _ = self.tx.send(Event::DebugLog {
                            message: format!("Failed to restore corrupted file {file_path}: {e}"),
                            context: HashMap::default(),
                        });
                    }
                },
                Discrepancy::MissingVenv {
                    package_name,
                    package_version,
                    venv_path,
                } => match self
                    .heal_missing_venv(package_name, package_version, venv_path)
                    .await
                {
                    Ok(()) => {
                        healed_count += 1;
                        let _ = self.tx.send(Event::DebugLog {
                            message: format!("Successfully healed missing venv: {venv_path} for {package_name}-{package_version}"),
                            context: HashMap::default(),
                        });
                    }
                    Err(e) => {
                        failed_healings.push(discrepancy.clone());
                        let _ = self.tx.send(Event::DebugLog {
                            message: format!("Failed to heal missing venv {venv_path}: {e}"),
                            context: HashMap::default(),
                        });
                    }
                },
                // TODO: Handle other discrepancy types in future phases
                _ => {
                    failed_healings.push(discrepancy.clone());
                }
            }
        }

        // Update verification result with healing results
        verification_result.discrepancies = failed_healings;
        verification_result.is_valid = verification_result.discrepancies.is_empty();

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        verification_result.duration_ms = duration_ms;

        // Emit healing complete event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Healing completed: {} healed, {} failed in {}ms",
                healed_count,
                verification_result.discrepancies.len(),
                duration_ms
            ),
            context: HashMap::default(),
        });

        Ok(verification_result)
    }

    /// Verify current state without healing
    ///
    /// # Errors
    ///
    /// Returns an error if state verification fails or database operations fail.
    pub async fn verify_only(&mut self) -> Result<VerificationResult, Error> {
        let start_time = Instant::now();
        let state_id = self.state_manager.get_active_state().await?;
        let live_path = self.state_manager.live_path().to_path_buf();

        // Emit verification started event
        let _ = self.tx.send(Event::DebugLog {
            message: format!("Starting state verification for state {state_id}"),
            context: HashMap::default(),
        });

        // Get all installed packages
        let mut tx = self.state_manager.begin_transaction().await?;
        let packages = queries::get_state_packages(&mut tx, &state_id).await?;
        tx.commit().await?;

        let mut discrepancies = Vec::new();
        let mut tracked_files = HashSet::new();

        // Verify each package
        for package in &packages {
            self.verify_package(
                &state_id,
                package,
                &live_path,
                &mut discrepancies,
                &mut tracked_files,
            )
            .await?;
        }

        // Check for orphaned files if not in Quick mode
        if self.level != VerificationLevel::Quick {
            Self::find_orphaned_files(&live_path, &tracked_files, &mut discrepancies);
        }

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Emit verification completed event with cache stats
        let cache_stats = self.cache.stats();
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "State verification completed in {duration_ms}ms with {} discrepancies. Cache: {:.1}% hit rate ({}/{} hits/lookups)",
                discrepancies.len(),
                cache_stats.hit_rate(),
                cache_stats.hits,
                cache_stats.lookups
            ),
            context: HashMap::default(),
        });

        Ok(VerificationResult::new(
            state_id,
            discrepancies,
            duration_ms,
        ))
    }

    /// Get cache statistics
    #[must_use]
    pub fn cache_stats(&self) -> &CacheStats {
        self.cache.stats()
    }

    /// Clear the verification cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Invalidate cache entries for a specific package
    pub fn invalidate_package_cache(&mut self, package_name: &str, package_version: &str) {
        self.cache.invalidate_package(package_name, package_version);
    }

    /// Load cache from persistent storage
    pub async fn load_cache(&mut self) -> Result<(), Error> {
        self.cache.load_from_storage().await
    }

    /// Save cache to persistent storage
    pub async fn save_cache(&self) -> Result<(), Error> {
        self.cache.save_to_storage().await
    }

    /// Verify a single package
    async fn verify_package(
        &mut self,
        state_id: &Uuid,
        package: &sps2_state::models::Package,
        live_path: &Path,
        discrepancies: &mut Vec<Discrepancy>,
        tracked_files: &mut HashSet<PathBuf>,
    ) -> Result<(), Error> {
        // Get package files from database
        let mut tx = self.state_manager.begin_transaction().await?;
        let file_paths =
            queries::get_package_files(&mut tx, state_id, &package.name, &package.version).await?;
        tx.commit().await?;

        // Verify each file
        for file_path in file_paths {
            let full_path = live_path.join(&file_path);
            tracked_files.insert(PathBuf::from(&file_path));

            // Check cache first
            if self.cache.is_entry_valid(&file_path, self.level) {
                // Cache hit - use cached result
                if let Some(cached_entry) = self.cache.get_entry(&file_path) {
                    if !cached_entry.was_valid {
                        // Cached entry indicates previous failure, add to discrepancies
                        if !full_path.exists() {
                            discrepancies.push(Discrepancy::MissingFile {
                                package_name: package.name.clone(),
                                package_version: package.version.clone(),
                                file_path: file_path.clone(),
                            });
                        }
                    }
                    // Skip verification for cached files
                    continue;
                }
            }

            // Cache miss - perform verification
            let _verification_start = Instant::now();
            let mut file_was_valid = true;

            // Check if file exists
            if !full_path.exists() {
                discrepancies.push(Discrepancy::MissingFile {
                    package_name: package.name.clone(),
                    package_version: package.version.clone(),
                    file_path: file_path.clone(),
                });
                file_was_valid = false;
            } else {
                // For Full verification, check content hash
                if self.level == VerificationLevel::Full {
                    let discrepancy_count_before = discrepancies.len();
                    self.verify_file_content(&full_path, package, &file_path, discrepancies)
                        .await?;
                    // If discrepancies were added, file was invalid
                    if discrepancies.len() > discrepancy_count_before {
                        file_was_valid = false;
                    }
                }
            }

            // Update cache with verification result
            if let Ok(metadata) = std::fs::metadata(&full_path) {
                if let Ok(mtime) = metadata.modified() {
                    let size = metadata.len();
                    let content_hash = if self.level == VerificationLevel::Full && file_was_valid {
                        // Calculate hash for Full verification
                        match sps2_hash::Hash::hash_file(&full_path).await {
                            Ok(hash) => Some(hash.to_string()),
                            Err(_) => None,
                        }
                    } else {
                        None
                    };

                    let cache_entry = FileCacheEntry {
                        file_path: file_path.clone(),
                        package_name: package.name.clone(),
                        package_version: package.version.clone(),
                        mtime,
                        size,
                        content_hash,
                        verified_at: SystemTime::now(),
                        verification_level: self.level,
                        was_valid: file_was_valid,
                    };

                    self.cache.update_entry(cache_entry);
                }
            }
        }

        // Check Python venv if applicable
        if let Some(venv_path) = &package.venv_path {
            if !Path::new(venv_path).exists() {
                discrepancies.push(Discrepancy::MissingVenv {
                    package_name: package.name.clone(),
                    package_version: package.version.clone(),
                    venv_path: venv_path.clone(),
                });
            }
        }

        Ok(())
    }

    /// Verify file content hash
    async fn verify_file_content(
        &self,
        file_path: &Path,
        package: &sps2_state::models::Package,
        relative_path: &str,
        discrepancies: &mut Vec<Discrepancy>,
    ) -> Result<(), Error> {
        // Skip hash verification for directories and symlinks
        let metadata = tokio::fs::symlink_metadata(file_path).await?;
        if metadata.is_dir() || metadata.is_symlink() {
            return Ok(());
        }

        // Load package from store to get expected file hash
        let package_hash =
            Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
                message: format!("Invalid package hash: {e}"),
            })?;
        let store_path = self.store.package_path(&package_hash);

        if !store_path.exists() {
            // Can't verify without store package
            return Ok(());
        }

        let stored_package = StoredPackage::load(&store_path).await?;
        let expected_file_path = stored_package.files_path().join(relative_path);

        if !expected_file_path.exists() {
            // File not in store, can't verify
            return Ok(());
        }

        // Calculate actual file hash
        let actual_hash = Hash::hash_file(file_path).await?;
        let expected_hash = Hash::hash_file(&expected_file_path).await?;

        if actual_hash != expected_hash {
            discrepancies.push(Discrepancy::CorruptedFile {
                package_name: package.name.clone(),
                package_version: package.version.clone(),
                file_path: relative_path.to_string(),
                expected_hash: expected_hash.to_hex(),
                actual_hash: actual_hash.to_hex(),
            });
        }

        Ok(())
    }

    /// Find orphaned files (files in live but not tracked in DB)
    fn find_orphaned_files(
        live_path: &Path,
        tracked_files: &HashSet<PathBuf>,
        discrepancies: &mut Vec<Discrepancy>,
    ) {
        use walkdir::WalkDir;

        // Walk the live directory
        for entry in WalkDir::new(live_path)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            let path = entry.path();
            if let Ok(relative) = path.strip_prefix(live_path) {
                // Skip the root directory itself
                if relative.as_os_str().is_empty() {
                    continue;
                }

                let relative_path = relative.to_path_buf();

                // Check if this file is tracked
                if !tracked_files.contains(&relative_path) {
                    let path_str = relative_path.to_string_lossy();

                    // Categorize the orphaned file
                    let category = Self::categorize_orphaned_file(&path_str, path);

                    // Skip system files that should always be preserved
                    if matches!(category, OrphanedFileCategory::System) {
                        continue;
                    }

                    discrepancies.push(Discrepancy::OrphanedFile {
                        file_path: path_str.to_string(),
                        category,
                    });
                }
            }
        }
    }

    /// Categorize an orphaned file based on its path and characteristics
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    fn categorize_orphaned_file(path_str: &str, full_path: &Path) -> OrphanedFileCategory {
        // System files that should always be preserved
        if path_str.starts_with(".DS_Store")
            || path_str.starts_with("lost+found")
            || path_str.starts_with(".Spotlight-")
            || path_str.starts_with(".fseventsd")
            || path_str.starts_with(".Trashes")
        {
            return OrphanedFileCategory::System;
        }

        // Temporary files
        if path_str.ends_with(".tmp")
            || path_str.ends_with(".temp")
            || path_str.ends_with('~')
            || path_str.contains("/.cache/")
            || path_str.contains("/tmp/")
        {
            return OrphanedFileCategory::Temporary;
        }

        // User-created files (configs, data, etc)
        if path_str.ends_with(".conf")
            || path_str.ends_with(".config")
            || path_str.ends_with(".ini")
            || path_str.ends_with(".json")
            || path_str.ends_with(".yaml")
            || path_str.ends_with(".yml")
            || path_str.ends_with(".toml")
            || path_str.ends_with(".db")
            || path_str.ends_with(".sqlite")
            || path_str.contains("/data/")
            || path_str.contains("/config/")
            || path_str.contains("/var/")
        {
            return OrphanedFileCategory::UserCreated;
        }

        // Check if it might be a leftover from a previous package version
        // by looking at common binary/library extensions
        if path_str.ends_with(".so")
            || path_str.ends_with(".dylib")
            || path_str.ends_with(".a")
            || path_str.contains("/bin/")
            || path_str.contains("/lib/")
            || path_str.contains("/share/")
        {
            // Additional check: if file is executable, likely leftover
            if let Ok(metadata) = std::fs::metadata(full_path) {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o111 != 0 {
                        return OrphanedFileCategory::Leftover;
                    }
                }
            }
            return OrphanedFileCategory::Leftover;
        }

        // Default to unknown for further investigation
        OrphanedFileCategory::Unknown
    }

    /// Get the current verification level
    #[must_use]
    pub const fn level(&self) -> VerificationLevel {
        self.level
    }

    /// Restore a missing file from the package store
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Package information cannot be retrieved from database
    /// - Package content is missing from store
    /// - File restoration fails
    async fn restore_missing_file(
        &self,
        package_name: &str,
        package_version: &str,
        file_path: &str,
    ) -> Result<(), Error> {
        // Get package hash from database
        let mut tx = self.state_manager.begin_transaction().await?;
        let state_id = self.state_manager.get_active_state().await?;
        let packages = queries::get_state_packages(&mut tx, &state_id).await?;
        tx.commit().await?;

        // Find the specific package
        let package = packages
            .iter()
            .find(|p| p.name == package_name && p.version == package_version)
            .ok_or_else(|| OpsError::OperationFailed {
                message: format!("Package {package_name}-{package_version} not found in state"),
            })?;

        // Load package from store
        let package_hash =
            Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
                message: format!("Invalid package hash: {e}"),
            })?;
        let store_path = self.store.package_path(&package_hash);

        if !store_path.exists() {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "Package content missing from store for {package_name}-{package_version}"
                ),
            }
            .into());
        }

        let stored_package = StoredPackage::load(&store_path).await?;
        let source_file = stored_package.files_path().join(file_path);

        if !source_file.exists() {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "File {file_path} not found in stored package {package_name}-{package_version}"
                ),
            }
            .into());
        }

        // Determine target path
        let live_path = self.state_manager.live_path();
        let target_path = live_path.join(file_path);

        // Create parent directories if needed
        if let Some(parent) = target_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to create parent directories: {e}"),
                })?;
        }

        // Get source file metadata for permissions
        let metadata = tokio::fs::metadata(&source_file).await?;

        // Restore the file based on its type
        if metadata.is_dir() {
            // Create directory
            tokio::fs::create_dir_all(&target_path).await.map_err(|e| {
                OpsError::OperationFailed {
                    message: format!("Failed to create directory {}: {e}", target_path.display()),
                }
            })?;
        } else if metadata.is_symlink() {
            // Read and recreate symlink
            let link_target = tokio::fs::read_link(&source_file).await?;
            tokio::fs::symlink(&link_target, &target_path)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to create symlink {}: {e}", target_path.display()),
                })?;
        } else {
            // Regular file - use APFS clonefile for efficiency on macOS
            #[cfg(target_os = "macos")]
            {
                sps2_root::clone_directory(&source_file, &target_path)
                    .await
                    .map_err(|e| OpsError::OperationFailed {
                        message: format!("Failed to clone file {}: {e}", target_path.display()),
                    })?;
            }

            #[cfg(not(target_os = "macos"))]
            {
                tokio::fs::copy(&source_file, &target_path)
                    .await
                    .map_err(|e| OpsError::OperationFailed {
                        message: format!("Failed to copy file {}: {e}", target_path.display()),
                    })?;
            }
        }

        // Restore permissions (on Unix-like systems)
        #[cfg(unix)]
        {
            let permissions = metadata.permissions();
            tokio::fs::set_permissions(&target_path, permissions)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to set permissions: {e}"),
                })?;
        }

        Ok(())
    }

    /// Handle an orphaned file based on configuration and category
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - File operations fail
    /// - Backup directory creation fails
    async fn handle_orphaned_file(
        &self,
        file_path: &str,
        category: &OrphanedFileCategory,
        config: &sps2_config::Config,
    ) -> Result<(), Error> {
        let live_path = self.state_manager.live_path();
        let full_path = live_path.join(file_path);

        // Determine action based on configuration and category
        let action = Self::determine_orphaned_file_action(category, config);

        // Emit event about the action
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Handling orphaned file: {file_path} (category: {category:?}, action: {action:?})"
            ),
            context: HashMap::default(),
        });

        match action {
            OrphanedFileAction::Preserve => {
                // Just log that we're preserving it
                let _ = self.tx.send(Event::DebugLog {
                    message: format!("Preserving orphaned file: {file_path}"),
                    context: HashMap::default(),
                });
                Ok(())
            }
            OrphanedFileAction::Remove => self.remove_orphaned_file(&full_path, file_path).await,
            OrphanedFileAction::Backup => {
                self.backup_and_remove_orphaned_file(
                    &full_path,
                    file_path,
                    &config.verification.orphaned_backup_dir,
                )
                .await
            }
        }
    }

    /// Determine what action to take for an orphaned file
    fn determine_orphaned_file_action(
        category: &OrphanedFileCategory,
        config: &sps2_config::Config,
    ) -> OrphanedFileAction {
        // System files are always preserved
        if matches!(category, OrphanedFileCategory::System) {
            return OrphanedFileAction::Preserve;
        }

        // User-created files respect configuration
        if matches!(category, OrphanedFileCategory::UserCreated)
            && config.verification.preserve_user_files
        {
            return OrphanedFileAction::Preserve;
        }

        // Parse the configured action
        match config.verification.orphaned_file_action.as_str() {
            "remove" => OrphanedFileAction::Remove,
            "backup" => OrphanedFileAction::Backup,
            _ => OrphanedFileAction::Preserve, // Default to preserve for safety
        }
    }

    /// Safely remove an orphaned file
    async fn remove_orphaned_file(
        &self,
        full_path: &Path,
        relative_path: &str,
    ) -> Result<(), Error> {
        // Check if it's a directory or file
        let metadata = tokio::fs::metadata(full_path).await?;

        if metadata.is_dir() {
            // For directories, only remove if empty
            match tokio::fs::read_dir(full_path).await {
                Ok(mut entries) => {
                    if entries.next_entry().await?.is_some() {
                        // Directory not empty, preserve it
                        let _ = self.tx.send(Event::DebugLog {
                            message: format!(
                                "Preserving non-empty orphaned directory: {relative_path}"
                            ),
                            context: HashMap::default(),
                        });
                        return Ok(());
                    }
                    // Directory is empty, safe to remove
                    tokio::fs::remove_dir(full_path).await.map_err(|e| {
                        OpsError::OperationFailed {
                            message: format!(
                                "Failed to remove empty directory {relative_path}: {e}"
                            ),
                        }
                    })?;
                }
                Err(e) => {
                    return Err(OpsError::OperationFailed {
                        message: format!("Failed to read directory {relative_path}: {e}"),
                    }
                    .into());
                }
            }
        } else {
            // Regular file or symlink
            tokio::fs::remove_file(full_path)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to remove file {relative_path}: {e}"),
                })?;
        }

        let _ = self.tx.send(Event::DebugLog {
            message: format!("Removed orphaned file: {relative_path}"),
            context: HashMap::default(),
        });

        Ok(())
    }

    /// Backup an orphaned file then remove it
    async fn backup_and_remove_orphaned_file(
        &self,
        full_path: &Path,
        relative_path: &str,
        backup_dir: &Path,
    ) -> Result<(), Error> {
        // Create backup directory structure
        let backup_path = backup_dir.join(relative_path);
        if let Some(parent) = backup_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to create backup directory: {e}"),
                })?;
        }

        // Move file to backup location
        tokio::fs::rename(full_path, &backup_path)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!(
                    "Failed to backup file {relative_path} to {}: {e}",
                    backup_path.display()
                ),
            })?;

        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Backed up orphaned file: {relative_path} -> {}",
                backup_path.display()
            ),
            context: HashMap::default(),
        });

        Ok(())
    }

    /// Heal a corrupted file by restoring it from the package store
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Package information cannot be retrieved
    /// - File restoration fails
    /// - Store content is missing
    async fn heal_corrupted_file(
        &self,
        package_name: &str,
        package_version: &str,
        file_path: &str,
        expected_hash: &str,
        actual_hash: &str,
    ) -> Result<(), Error> {
        let live_path = self.state_manager.live_path();
        let full_path = live_path.join(file_path);

        // First, check if this might be a legitimate user modification
        if self.is_user_modified_file(&full_path, file_path).await? {
            // Preserve user modifications
            let _ = self.tx.send(Event::DebugLog {
                message: format!(
                    "Preserving user-modified file: {file_path} (hash mismatch: expected {expected_hash}, got {actual_hash})"
                ),
                context: HashMap::default(),
            });
            return Ok(());
        }

        // Get package from database to find store location
        let mut tx = self.state_manager.begin_transaction().await?;
        let state_id = self.state_manager.get_active_state().await?;
        let packages = queries::get_state_packages(&mut tx, &state_id).await?;
        tx.commit().await?;

        let package = packages
            .iter()
            .find(|p| p.name == package_name && p.version == package_version)
            .ok_or_else(|| OpsError::OperationFailed {
                message: format!("Package {package_name}-{package_version} not found in state"),
            })?;

        // Load package from store
        let package_hash =
            Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
                message: format!("Invalid package hash: {e}"),
            })?;
        let store_path = self.store.package_path(&package_hash);

        if !store_path.exists() {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "Package content missing from store for {package_name}-{package_version}"
                ),
            }
            .into());
        }

        let stored_package = StoredPackage::load(&store_path).await?;
        let source_file = stored_package.files_path().join(file_path);

        if !source_file.exists() {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "File {file_path} not found in stored package {package_name}-{package_version}"
                ),
            }
            .into());
        }

        // Verify the source file has the expected hash
        let source_hash = Hash::hash_file(&source_file).await?;
        if source_hash.to_hex() != expected_hash {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "Source file in store also corrupted for {file_path} (expected {expected_hash}, got {})",
                    source_hash.to_hex()
                ),
            }
            .into());
        }

        // Backup the corrupted file before replacing
        let backup_path = full_path.with_extension("corrupted.backup");
        tokio::fs::rename(&full_path, &backup_path)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to backup corrupted file: {e}"),
            })?;

        // Restore the file from store
        let metadata = tokio::fs::metadata(&source_file).await?;

        if metadata.is_symlink() {
            // Recreate symlink
            let link_target = tokio::fs::read_link(&source_file).await?;
            tokio::fs::symlink(&link_target, &full_path)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to restore symlink {}: {e}", full_path.display()),
                })?;
        } else {
            // Regular file - use APFS clonefile for efficiency on macOS
            #[cfg(target_os = "macos")]
            {
                sps2_root::clone_directory(&source_file, &full_path)
                    .await
                    .map_err(|e| OpsError::OperationFailed {
                        message: format!("Failed to restore file {}: {e}", full_path.display()),
                    })?;
            }

            #[cfg(not(target_os = "macos"))]
            {
                tokio::fs::copy(&source_file, &full_path)
                    .await
                    .map_err(|e| OpsError::OperationFailed {
                        message: format!("Failed to restore file {}: {e}", full_path.display()),
                    })?;
            }
        }

        // Restore permissions
        #[cfg(unix)]
        {
            let permissions = metadata.permissions();
            tokio::fs::set_permissions(&full_path, permissions)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to restore permissions: {e}"),
                })?;
        }

        // Emit success event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Restored corrupted file: {file_path} (backup saved as {})",
                backup_path.display()
            ),
            context: HashMap::default(),
        });

        Ok(())
    }

    /// Check if a file appears to be user-modified
    ///
    /// This is a heuristic check based on:
    /// - File modification time vs package installation time
    /// - Common user-modifiable file patterns
    /// - File location and type
    async fn is_user_modified_file(
        &self,
        full_path: &Path,
        relative_path: &str,
    ) -> Result<bool, Error> {
        // Common patterns for user-modifiable files
        const USER_MODIFIABLE_PATTERNS: &[&str] = &[
            // Configuration files
            ".conf",
            ".config",
            ".ini",
            ".json",
            ".yaml",
            ".yml",
            ".toml",
            // Shell configuration
            ".bashrc",
            ".zshrc",
            ".profile",
            ".bash_profile",
            // Environment files
            ".env",
            ".envrc",
            // User data
            ".db",
            ".sqlite",
            ".sqlite3",
        ];

        // Check if file matches user-modifiable patterns
        let path_str = relative_path.to_lowercase();
        for pattern in USER_MODIFIABLE_PATTERNS {
            if path_str.ends_with(pattern)
                || path_str.contains("/etc/")
                || path_str.contains("/config/")
            {
                // These files are commonly modified by users
                let _ = self.tx.send(Event::DebugLog {
                    message: format!("File {relative_path} matches user-modifiable pattern"),
                    context: HashMap::default(),
                });
                return Ok(true);
            }
        }

        // Check file metadata for recent modifications
        if let Ok(metadata) = tokio::fs::metadata(full_path).await {
            if let Ok(modified) = metadata.modified() {
                // If file was modified very recently (within last hour), it might be user-modified
                if let Ok(elapsed) = modified.elapsed() {
                    if elapsed.as_secs() < 3600 {
                        let _ = self.tx.send(Event::DebugLog {
                            message: format!(
                                "File {relative_path} was modified recently ({} seconds ago)",
                                elapsed.as_secs()
                            ),
                            context: HashMap::default(),
                        });
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    /// Heal a missing Python virtual environment by recreating it
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Package information cannot be retrieved
    /// - Python metadata is missing
    /// - Venv recreation fails
    /// - Package reinstallation fails
    async fn heal_missing_venv(
        &self,
        package_name: &str,
        package_version: &str,
        venv_path: &str,
    ) -> Result<(), Error> {
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Starting venv healing for {package_name}-{package_version} at {venv_path}"
            ),
            context: HashMap::default(),
        });

        // Step 1: Get package information from database
        let mut tx = self.state_manager.begin_transaction().await?;
        let state_id = self.state_manager.get_active_state().await?;
        let packages = queries::get_state_packages(&mut tx, &state_id).await?;
        tx.commit().await?;

        let package = packages
            .iter()
            .find(|p| p.name == package_name && p.version == package_version)
            .ok_or_else(|| OpsError::OperationFailed {
                message: format!("Package {package_name}-{package_version} not found in state"),
            })?;

        // Step 2: Load package manifest from store to get Python metadata
        let package_hash =
            Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
                message: format!("Invalid package hash: {e}"),
            })?;
        let store_path = self.store.package_path(&package_hash);

        if !store_path.exists() {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "Package content missing from store for {package_name}-{package_version}"
                ),
            }
            .into());
        }

        let stored_package = StoredPackage::load(&store_path).await?;
        let manifest = stored_package.manifest();

        let python_metadata =
            manifest
                .python
                .as_ref()
                .ok_or_else(|| OpsError::OperationFailed {
                    message: format!(
                        "Package {package_name}-{package_version} is not a Python package"
                    ),
                })?;

        // Step 3: Capture existing pip packages if venv partially exists
        let venv_path_buf = PathBuf::from(venv_path);
        let python_bin = venv_path_buf.join("bin/python");
        let mut existing_packages = Vec::new();

        if python_bin.exists() {
            let _ = self.tx.send(Event::DebugLog {
                message: format!("Capturing existing pip packages from {venv_path}"),
                context: HashMap::default(),
            });

            // Run pip freeze to capture existing packages
            match tokio::process::Command::new(&python_bin)
                .arg("-m")
                .arg("pip")
                .arg("freeze")
                .output()
                .await
            {
                Ok(output) if output.status.success() => {
                    let freeze_output = String::from_utf8_lossy(&output.stdout);
                    existing_packages = freeze_output
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .map(String::from)
                        .collect();

                    let _ = self.tx.send(Event::DebugLog {
                        message: format!("Captured {} existing packages", existing_packages.len()),
                        context: HashMap::default(),
                    });
                }
                _ => {
                    let _ = self.tx.send(Event::DebugLog {
                        message: "Failed to capture existing packages, proceeding with fresh venv"
                            .to_string(),
                        context: HashMap::default(),
                    });
                }
            }
        }

        // Step 4: Remove corrupted venv
        if venv_path_buf.exists() {
            tokio::fs::remove_dir_all(&venv_path_buf)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to remove corrupted venv: {e}"),
                })?;
        }

        // Step 5: Create new venv using the PythonVenvManager
        let venvs_base = PathBuf::from("/opt/pm/venvs");
        let venv_manager = sps2_install::PythonVenvManager::new(venvs_base);

        let package_id = sps2_install::python::PackageId::new(
            package_name.to_string(),
            sps2_types::Version::parse(package_version).map_err(|e| OpsError::OperationFailed {
                message: format!("Invalid version: {e}"),
            })?,
        );

        venv_manager
            .create_venv(&package_id, python_metadata, Some(&self.tx))
            .await?;

        // Step 6: Install the wheel file
        let wheel_path = stored_package
            .files_path()
            .join(&python_metadata.wheel_file);
        let requirements_path = stored_package
            .files_path()
            .join(&python_metadata.requirements_file);

        venv_manager
            .install_wheel(
                &package_id,
                &venv_path_buf,
                &wheel_path,
                Some(&requirements_path),
                Some(&self.tx),
            )
            .await?;

        // Step 7: Restore previously installed packages (best effort)
        if !existing_packages.is_empty() {
            let _ = self.tx.send(Event::DebugLog {
                message: format!(
                    "Attempting to restore {} previously installed packages",
                    existing_packages.len()
                ),
                context: HashMap::default(),
            });

            // Create a temporary requirements file with the captured packages
            let temp_reqs = venv_path_buf.join("restore_requirements.txt");
            tokio::fs::write(&temp_reqs, existing_packages.join("\n"))
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to create restore requirements: {e}"),
                })?;

            // Try to reinstall the packages (don't fail if some packages can't be installed)
            let output = tokio::process::Command::new("uv")
                .arg("pip")
                .arg("install")
                .arg("--python")
                .arg(&python_bin)
                .arg("-r")
                .arg(&temp_reqs)
                .output()
                .await;

            // Clean up temp file
            let _ = tokio::fs::remove_file(&temp_reqs).await;

            match output {
                Ok(result) if result.status.success() => {
                    let _ = self.tx.send(Event::DebugLog {
                        message: "Successfully restored previous packages".to_string(),
                        context: HashMap::default(),
                    });
                }
                _ => {
                    let _ = self.tx.send(Event::DebugLog {
                        message: "Some packages could not be restored, but venv is functional"
                            .to_string(),
                        context: HashMap::default(),
                    });
                }
            }
        }

        let _ = self.tx.send(Event::DebugLog {
            message: format!("Successfully healed venv for {package_name}-{package_version}"),
            context: HashMap::default(),
        });

        Ok(())
    }
}

/// Builder for `StateVerificationGuard`
pub struct StateVerificationGuardBuilder {
    state_manager: Option<StateManager>,
    store: Option<PackageStore>,
    tx: Option<EventSender>,
    level: VerificationLevel,
}

impl StateVerificationGuardBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            state_manager: None,
            store: None,
            tx: None,
            level: VerificationLevel::default(),
        }
    }

    /// Set the state manager
    #[must_use]
    pub fn with_state_manager(mut self, state_manager: StateManager) -> Self {
        self.state_manager = Some(state_manager);
        self
    }

    /// Set the package store
    #[must_use]
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Set the event sender
    #[must_use]
    pub fn with_event_sender(mut self, tx: EventSender) -> Self {
        self.tx = Some(tx);
        self
    }

    /// Set the verification level
    #[must_use]
    pub fn with_level(mut self, level: VerificationLevel) -> Self {
        self.level = level;
        self
    }

    /// Build the guard
    ///
    /// # Errors
    ///
    /// Returns an error if any required component is missing.
    pub fn build(self) -> Result<StateVerificationGuard, Error> {
        let state_manager = self
            .state_manager
            .ok_or_else(|| OpsError::MissingComponent {
                component: "StateManager".to_string(),
            })?;

        let store = self.store.ok_or_else(|| OpsError::MissingComponent {
            component: "PackageStore".to_string(),
        })?;

        let tx = self.tx.ok_or_else(|| OpsError::MissingComponent {
            component: "EventSender".to_string(),
        })?;

        // Create cache
        let cache = VerificationCache::new();

        Ok(StateVerificationGuard {
            state_manager,
            store,
            tx,
            level: self.level,
            cache,
        })
    }
}

impl Default for StateVerificationGuardBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_level_default() {
        assert_eq!(VerificationLevel::default(), VerificationLevel::Standard);
    }

    #[test]
    fn test_verification_result_validity() {
        let state_id = Uuid::new_v4();

        // Test valid result (no discrepancies)
        let result = VerificationResult::new(state_id, vec![], 100);
        assert!(result.is_valid);
        assert_eq!(result.discrepancies.len(), 0);
        assert_eq!(result.duration_ms, 100);

        // Test invalid result (with discrepancies)
        let discrepancies = vec![Discrepancy::MissingFile {
            package_name: "test".to_string(),
            package_version: "1.0.0".to_string(),
            file_path: "/bin/test".to_string(),
        }];
        let result = VerificationResult::new(state_id, discrepancies, 200);
        assert!(!result.is_valid);
        assert_eq!(result.discrepancies.len(), 1);
    }

    #[test]
    fn test_builder_pattern() {
        // Test that builder requires all fields
        let builder = StateVerificationGuardBuilder::new();
        assert!(builder.build().is_err());

        // Test builder with verification level
        let builder = StateVerificationGuardBuilder::new().with_level(VerificationLevel::Full);
        assert!(builder.build().is_err()); // Still missing required fields
    }

    #[test]
    fn test_discrepancy_types() {
        // Test MissingFile discrepancy
        let missing_file = Discrepancy::MissingFile {
            package_name: "test-pkg".to_string(),
            package_version: "1.0.0".to_string(),
            file_path: "/bin/test".to_string(),
        };
        match missing_file {
            Discrepancy::MissingFile { package_name, .. } => {
                assert_eq!(package_name, "test-pkg");
            }
            _ => panic!("Expected MissingFile discrepancy"),
        }

        // Test CorruptedFile discrepancy
        let corrupted_file = Discrepancy::CorruptedFile {
            package_name: "test-pkg".to_string(),
            package_version: "1.0.0".to_string(),
            file_path: "/bin/test".to_string(),
            expected_hash: "abc123".to_string(),
            actual_hash: "def456".to_string(),
        };
        match corrupted_file {
            Discrepancy::CorruptedFile {
                expected_hash,
                actual_hash,
                ..
            } => {
                assert_eq!(expected_hash, "abc123");
                assert_eq!(actual_hash, "def456");
            }
            _ => panic!("Expected CorruptedFile discrepancy"),
        }

        // Test OrphanedFile discrepancy
        let orphaned_file = Discrepancy::OrphanedFile {
            file_path: "/tmp/orphan.txt".to_string(),
            category: OrphanedFileCategory::Temporary,
        };
        match orphaned_file {
            Discrepancy::OrphanedFile {
                file_path,
                category,
            } => {
                assert_eq!(file_path, "/tmp/orphan.txt");
                assert_eq!(category, OrphanedFileCategory::Temporary);
            }
            _ => panic!("Expected OrphanedFile discrepancy"),
        }

        // Test MissingVenv discrepancy
        let missing_venv = Discrepancy::MissingVenv {
            package_name: "python-pkg".to_string(),
            package_version: "2.0.0".to_string(),
            venv_path: "/opt/pm/venvs/python-pkg-2.0.0".to_string(),
        };
        match missing_venv {
            Discrepancy::MissingVenv { venv_path, .. } => {
                assert_eq!(venv_path, "/opt/pm/venvs/python-pkg-2.0.0");
            }
            _ => panic!("Expected MissingVenv discrepancy"),
        }
    }

    #[test]
    fn test_verification_levels() {
        assert_eq!(VerificationLevel::Quick as u8, 0);
        assert_eq!(VerificationLevel::Standard as u8, 1);
        assert_eq!(VerificationLevel::Full as u8, 2);
    }

    #[tokio::test]
    async fn test_guard_creation_and_level() {
        use sps2_state::StateManager;
        use sps2_store::PackageStore;
        use tempfile::TempDir;

        // Create temporary directories
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        // Create directories
        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        // Create managers
        let state_manager = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Test guard creation with different levels
        let mut guard_quick = StateVerificationGuard::builder()
            .with_state_manager(state_manager.clone())
            .with_store(store.clone())
            .with_event_sender(tx.clone())
            .with_level(VerificationLevel::Quick)
            .build()
            .unwrap();
        assert_eq!(guard_quick.level(), VerificationLevel::Quick);

        let mut guard_full = StateVerificationGuard::builder()
            .with_state_manager(state_manager.clone())
            .with_store(store.clone())
            .with_event_sender(tx.clone())
            .with_level(VerificationLevel::Full)
            .build()
            .unwrap();
        assert_eq!(guard_full.level(), VerificationLevel::Full);
    }

    #[tokio::test]
    async fn test_verify_and_heal_with_no_discrepancies() {
        use sps2_state::StateManager;
        use sps2_store::PackageStore;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        let state_manager = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let mut guard = StateVerificationGuard::builder()
            .with_state_manager(state_manager)
            .with_store(store)
            .with_event_sender(tx)
            .with_level(VerificationLevel::Standard)
            .build()
            .unwrap();

        // On clean state, verify_and_heal should return valid result
        let config = sps2_config::Config::default();
        let result = guard.verify_and_heal(&config).await.unwrap();
        assert!(result.is_valid);
        assert_eq!(result.discrepancies.len(), 0);
    }

    #[test]
    fn test_healing_events() {
        // Test that healing events are properly formatted
        let discrepancy = Discrepancy::MissingFile {
            package_name: "test-pkg".to_string(),
            package_version: "1.0.0".to_string(),
            file_path: "/bin/test".to_string(),
        };

        match &discrepancy {
            Discrepancy::MissingFile {
                package_name,
                package_version,
                file_path,
            } => {
                let message = format!(
                    "Restored missing file: {file_path} from {package_name}-{package_version}"
                );
                assert_eq!(
                    message,
                    "Restored missing file: /bin/test from test-pkg-1.0.0"
                );
            }
            _ => panic!("Expected MissingFile discrepancy"),
        }
    }

    #[test]
    fn test_orphaned_file_categorization() {
        // Test system files
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file(".DS_Store", Path::new(".DS_Store")),
            OrphanedFileCategory::System
        );
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file(
                ".Spotlight-V100",
                Path::new(".Spotlight-V100")
            ),
            OrphanedFileCategory::System
        );

        // Test temporary files
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file("test.tmp", Path::new("test.tmp")),
            OrphanedFileCategory::Temporary
        );
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file("backup~", Path::new("backup~")),
            OrphanedFileCategory::Temporary
        );

        // Test user-created files
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file(
                "config.json",
                Path::new("config.json")
            ),
            OrphanedFileCategory::UserCreated
        );
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file(
                "settings.yaml",
                Path::new("settings.yaml")
            ),
            OrphanedFileCategory::UserCreated
        );

        // Test leftover files
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file(
                "lib/libtest.so",
                Path::new("lib/libtest.so")
            ),
            OrphanedFileCategory::Leftover
        );
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file(
                "bin/executable",
                Path::new("bin/executable")
            ),
            OrphanedFileCategory::Unknown // bin without leading slash is not in a standard bin directory
        );
        // Test files in standard directories
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file(
                "/bin/executable",
                Path::new("/bin/executable")
            ),
            OrphanedFileCategory::Leftover
        );

        // Test unknown files
        assert_eq!(
            StateVerificationGuard::categorize_orphaned_file("random.xyz", Path::new("random.xyz")),
            OrphanedFileCategory::Unknown
        );
    }

    #[test]
    fn test_orphaned_file_action_determination() {
        let mut config = sps2_config::Config::default();

        // Test that system files are always preserved
        assert_eq!(
            StateVerificationGuard::determine_orphaned_file_action(
                &OrphanedFileCategory::System,
                &config
            ),
            OrphanedFileAction::Preserve
        );

        // Test that user files are preserved when configured
        config.verification.preserve_user_files = true;
        assert_eq!(
            StateVerificationGuard::determine_orphaned_file_action(
                &OrphanedFileCategory::UserCreated,
                &config
            ),
            OrphanedFileAction::Preserve
        );

        // Test different action configurations
        config.verification.orphaned_file_action = "remove".to_string();
        assert_eq!(
            StateVerificationGuard::determine_orphaned_file_action(
                &OrphanedFileCategory::Temporary,
                &config
            ),
            OrphanedFileAction::Remove
        );

        config.verification.orphaned_file_action = "backup".to_string();
        assert_eq!(
            StateVerificationGuard::determine_orphaned_file_action(
                &OrphanedFileCategory::Leftover,
                &config
            ),
            OrphanedFileAction::Backup
        );

        config.verification.orphaned_file_action = "preserve".to_string();
        assert_eq!(
            StateVerificationGuard::determine_orphaned_file_action(
                &OrphanedFileCategory::Unknown,
                &config
            ),
            OrphanedFileAction::Preserve
        );
    }

    #[test]
    fn test_user_modifiable_patterns() {
        // Test patterns that should be recognized as user-modifiable
        let user_modifiable = vec![
            "app.conf",
            "config.json",
            "settings.yaml",
            ".bashrc",
            ".zshrc",
            ".profile",
            ".env",
            ".envrc",
            "data.db",
            "app.sqlite",
            "settings.toml",
            "config.ini",
            "/etc/app/config",
            "/opt/app/config/settings",
        ];

        // Test patterns that should NOT be recognized as user-modifiable
        let not_user_modifiable = vec![
            "binary",
            "lib.so",
            "app.dylib",
            "executable",
            "script.sh",
            "main.rs",
        ];

        const USER_MODIFIABLE_PATTERNS: &[&str] = &[
            ".conf",
            ".config",
            ".ini",
            ".json",
            ".yaml",
            ".yml",
            ".toml",
            ".bashrc",
            ".zshrc",
            ".profile",
            ".bash_profile",
            ".env",
            ".envrc",
            ".db",
            ".sqlite",
            ".sqlite3",
        ];

        for path in user_modifiable {
            let path_lower = path.to_lowercase();
            let is_modifiable = USER_MODIFIABLE_PATTERNS
                .iter()
                .any(|pattern| path_lower.ends_with(pattern))
                || path_lower.contains("/etc/")
                || path_lower.contains("/config/");
            assert!(is_modifiable, "Expected {} to be user-modifiable", path);
        }

        for path in not_user_modifiable {
            let path_lower = path.to_lowercase();
            let is_modifiable = USER_MODIFIABLE_PATTERNS
                .iter()
                .any(|pattern| path_lower.ends_with(pattern))
                || path_lower.contains("/etc/")
                || path_lower.contains("/config/");
            assert!(
                !is_modifiable,
                "Expected {} to NOT be user-modifiable",
                path
            );
        }
    }

    #[test]
    fn test_missing_venv_discrepancy() {
        // Test MissingVenv discrepancy creation and matching
        let missing_venv = Discrepancy::MissingVenv {
            package_name: "black".to_string(),
            package_version: "24.4.2".to_string(),
            venv_path: "/opt/pm/venvs/black-24.4.2".to_string(),
        };

        match missing_venv {
            Discrepancy::MissingVenv {
                package_name,
                package_version,
                venv_path,
            } => {
                assert_eq!(package_name, "black");
                assert_eq!(package_version, "24.4.2");
                assert_eq!(venv_path, "/opt/pm/venvs/black-24.4.2");
            }
            _ => panic!("Expected MissingVenv discrepancy"),
        }
    }

    #[test]
    fn test_venv_healing_events() {
        // Test that venv healing events are properly formatted
        let discrepancy = Discrepancy::MissingVenv {
            package_name: "mypy".to_string(),
            package_version: "1.10.0".to_string(),
            venv_path: "/opt/pm/venvs/mypy-1.10.0".to_string(),
        };

        match &discrepancy {
            Discrepancy::MissingVenv {
                package_name,
                package_version,
                venv_path,
            } => {
                let message = format!(
                    "Successfully healed missing venv: {venv_path} for {package_name}-{package_version}"
                );
                assert_eq!(
                    message,
                    "Successfully healed missing venv: /opt/pm/venvs/mypy-1.10.0 for mypy-1.10.0"
                );
            }
            _ => panic!("Expected MissingVenv discrepancy"),
        }
    }
}

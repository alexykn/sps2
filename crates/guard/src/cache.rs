//! Verification cache for performance optimization

use crate::types::{CacheStats, FileCacheEntry, VerificationLevel};
use sps2_errors::Error;
use std::collections::HashMap;
use std::time::SystemTime;

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

    /// Invalidate all cache entries for all versions of a package
    ///
    /// This removes all cached verification results for any version of the specified package,
    /// which is useful when a package is completely removed from the system.
    pub fn invalidate_package_all_versions(&mut self, package_name: &str) {
        self.entries.retain(|_path, entry| entry.package_name != package_name);
        self.update_stats();
    }

    /// Invalidate cache entries for files in a specific directory tree
    ///
    /// This removes all cached verification results for files that are children
    /// of the specified directory, which is useful when directory structures change.
    pub fn invalidate_directory(&mut self, directory: &std::path::Path) {
        let dir_str = directory.to_string_lossy();
        self.entries.retain(|path, _entry| {
            !std::path::Path::new(path).starts_with(directory) &&
            !path.starts_with(&dir_str.to_string())
        });
        self.update_stats();
    }

    /// Get the number of cache entries currently stored
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

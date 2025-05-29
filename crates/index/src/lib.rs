#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Package repository index for spsv2
//!
//! This crate handles the repository index that lists all available
//! packages and their versions. The index is cached locally for
//! offline use and validated for freshness.

mod cache;
mod models;

pub use cache::IndexCache;
pub use models::{
    DependencyInfo, Index, IndexMetadata, PackageEntry, SbomEntry, SbomInfo, VersionEntry,
};

use chrono::Utc;
use spsv2_errors::Error;
use spsv2_types::{Version, package::PackageSpec};
// HashMap removed - not used
use std::path::Path;

/// Supported index format version
pub const SUPPORTED_INDEX_VERSION: u32 = 1;

/// Repository index manager
#[derive(Clone)]
pub struct IndexManager {
    index: Option<Index>,
    cache: IndexCache,
}

impl IndexManager {
    /// Create a new index manager with cache directory
    pub fn new(cache_dir: impl AsRef<Path>) -> Self {
        Self {
            index: None,
            cache: IndexCache::new(cache_dir),
        }
    }

    /// Load index from cache or JSON content
    pub async fn load(&mut self, content: Option<&str>) -> Result<(), Error> {
        let index = if let Some(json) = content {
            // Parse provided content
            Index::from_json(json)?
        } else {
            // Try to load from cache
            self.cache.load().await?
        };

        // Validate index
        index.validate()?;

        self.index = Some(index);
        Ok(())
    }

    /// Save current index to cache
    pub async fn save_to_cache(&self) -> Result<(), Error> {
        if let Some(index) = &self.index {
            self.cache.save(index).await?;
        }
        Ok(())
    }

    /// Get the loaded index
    pub fn index(&self) -> Option<&Index> {
        self.index.as_ref()
    }

    /// Search for packages by name (prefix match)
    pub fn search(&self, query: &str) -> Vec<&str> {
        let Some(index) = &self.index else {
            return Vec::new();
        };

        let query_lower = query.to_lowercase();
        let mut results: Vec<&str> = index
            .packages
            .keys()
            .filter(|name| name.to_lowercase().starts_with(&query_lower))
            .map(String::as_str)
            .collect();

        results.sort_unstable();
        results
    }

    /// Get all versions of a package
    pub fn get_package_versions(&self, name: &str) -> Option<Vec<&VersionEntry>> {
        let index = self.index.as_ref()?;
        let package = index.packages.get(name)?;

        let mut versions: Vec<&VersionEntry> = package.versions.values().collect();
        versions.sort_by(|a, b| {
            // Sort by version descending (newest first)
            Version::parse(&b.version())
                .unwrap_or_else(|_| Version::new(0, 0, 0))
                .cmp(&Version::parse(&a.version()).unwrap_or_else(|_| Version::new(0, 0, 0)))
        });

        Some(versions)
    }

    /// Find the best version matching a spec
    pub fn find_best_version(&self, spec: &PackageSpec) -> Option<&VersionEntry> {
        let versions = self.get_package_versions(&spec.name)?;

        // Find highest version that satisfies the spec
        versions.into_iter().find(|v| {
            if let Ok(version) = Version::parse(&v.version()) {
                spec.version_spec.matches(&version)
            } else {
                false
            }
        })
    }

    /// Get a specific version entry
    pub fn get_version(&self, name: &str, version: &str) -> Option<&VersionEntry> {
        self.index
            .as_ref()?
            .packages
            .get(name)?
            .versions
            .get(version)
    }

    /// Check if index is stale (older than max_age_days)
    pub fn is_stale(&self, max_age_days: u32) -> bool {
        let Some(index) = &self.index else {
            return true;
        };

        let max_age = chrono::Duration::days(i64::from(max_age_days));
        let age = Utc::now() - index.metadata.timestamp;

        age > max_age
    }

    /// Get index metadata
    pub fn metadata(&self) -> Option<&IndexMetadata> {
        self.index.as_ref().map(|i| &i.metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_index_loading() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let json = r#"{
            "version": 1,
            "minimum_client": "0.1.0",
            "timestamp": "2025-05-29T12:00:00Z",
            "packages": {
                "test-pkg": {
                    "versions": {
                        "1.0.0": {
                            "revision": 1,
                            "arch": "arm64",
                            "sha256": "abc123",
                            "download_url": "https://example.com/test-pkg-1.0.0.sp",
                            "minisig_url": "https://example.com/test-pkg-1.0.0.sp.minisig",
                            "dependencies": {
                                "runtime": ["dep1>=1.0"],
                                "build": []
                            }
                        }
                    }
                }
            }
        }"#;

        manager.load(Some(json)).await.unwrap();

        let index = manager.index().unwrap();
        assert_eq!(index.metadata.version, 1);
        assert!(index.packages.contains_key("test-pkg"));
    }

    #[test]
    fn test_search() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        // Create test index
        let mut index = Index::new();
        index
            .packages
            .insert("curl".to_string(), PackageEntry::default());
        index
            .packages
            .insert("curlie".to_string(), PackageEntry::default());
        index
            .packages
            .insert("wget".to_string(), PackageEntry::default());

        manager.index = Some(index);

        // Test search
        let results = manager.search("cur");
        assert_eq!(results, vec!["curl", "curlie"]);

        let results = manager.search("CURL");
        assert_eq!(results, vec!["curl", "curlie"]);
    }
}

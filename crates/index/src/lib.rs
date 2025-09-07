#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Package repository index for sps2
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
use sps2_errors::Error;
use sps2_types::{package::PackageSpec, Version};
// HashMap removed - not used
use std::path::Path;

/// Supported index format version
pub const SUPPORTED_INDEX_VERSION: u32 = 1;

/// Repository index manager
#[derive(Clone, Debug)]
pub struct IndexManager {
    index: Option<Index>,
    pub cache: IndexCache,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON content is invalid, cache cannot be read,
    /// or the index fails validation.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the cache cannot be written to disk.
    pub async fn save_to_cache(&self) -> Result<(), Error> {
        if let Some(index) = &self.index {
            self.cache.save(index).await?;
        }
        Ok(())
    }

    /// Get the loaded index
    #[must_use]
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
    #[must_use]
    pub fn get_package_versions(&self, name: &str) -> Option<Vec<&VersionEntry>> {
        let index = self.index.as_ref()?;
        let package = index.packages.get(name)?;

        let mut versions: Vec<(&String, &VersionEntry)> = package.versions.iter().collect();
        versions.sort_by(|a, b| {
            // Sort by version descending (newest first)
            Version::parse(b.0)
                .unwrap_or_else(|_| Version::new(0, 0, 0))
                .cmp(&Version::parse(a.0).unwrap_or_else(|_| Version::new(0, 0, 0)))
        });

        Some(versions.into_iter().map(|(_, entry)| entry).collect())
    }

    /// Get all versions of a package, including their version strings
    /// sorted by version descending (newest first)
    #[must_use]
    pub fn get_package_versions_with_strings(
        &self,
        name: &str,
    ) -> Option<Vec<(&str, &VersionEntry)>> {
        let index = self.index.as_ref()?;
        let package = index.packages.get(name)?;

        let mut versions: Vec<(&String, &VersionEntry)> = package.versions.iter().collect();
        versions.sort_by(|a, b| {
            // Sort by version descending (newest first)
            Version::parse(b.0)
                .unwrap_or_else(|_| Version::new(0, 0, 0))
                .cmp(&Version::parse(a.0).unwrap_or_else(|_| Version::new(0, 0, 0)))
        });

        Some(
            versions
                .into_iter()
                .map(|(v, entry)| (v.as_str(), entry))
                .collect(),
        )
    }

    /// Find the best version matching a spec
    #[must_use]
    pub fn find_best_version(&self, spec: &PackageSpec) -> Option<&VersionEntry> {
        self.find_best_version_with_string(spec)
            .map(|(_, entry)| entry)
    }

    /// Find the best version matching a spec, returning both version string and entry
    #[must_use]
    pub fn find_best_version_with_string(
        &self,
        spec: &PackageSpec,
    ) -> Option<(&str, &VersionEntry)> {
        let index = self.index.as_ref()?;
        let package = index.packages.get(&spec.name)?;

        // Collect versions with their version strings, sort by version descending
        let mut versions: Vec<(&String, &VersionEntry)> = package.versions.iter().collect();
        versions.sort_by(|a, b| {
            // Sort by version descending (newest first)
            Version::parse(b.0)
                .unwrap_or_else(|_| Version::new(0, 0, 0))
                .cmp(&Version::parse(a.0).unwrap_or_else(|_| Version::new(0, 0, 0)))
        });

        // Find highest version that satisfies the spec
        versions.into_iter().find_map(|(version_str, entry)| {
            if let Ok(version) = Version::parse(version_str) {
                if spec.version_spec.matches(&version) {
                    Some((version_str.as_str(), entry))
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    /// Get a specific version entry
    #[must_use]
    pub fn get_version(&self, name: &str, version: &str) -> Option<&VersionEntry> {
        self.index
            .as_ref()?
            .packages
            .get(name)?
            .versions
            .get(version)
    }

    /// Check if index is stale (older than `max_age_days`)
    #[must_use]
    pub fn is_stale(&self, max_age_days: u32) -> bool {
        let Some(index) = &self.index else {
            return true;
        };

        let max_age = chrono::Duration::days(i64::from(max_age_days));
        let age = Utc::now() - index.metadata.timestamp;

        age > max_age
    }

    /// Get index metadata
    #[must_use]
    pub fn metadata(&self) -> Option<&IndexMetadata> {
        self.index.as_ref().map(|i| &i.metadata)
    }

    /// Set index directly (primarily for testing)
    ///
    /// This method bypasses validation and should only be used in tests.
    #[doc(hidden)]
    pub fn set_index(&mut self, index: Index) {
        self.index = Some(index);
    }
}

#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Content-addressed storage for spsv2
//!
//! This crate manages the `/opt/pm/store/` directory where packages
//! are stored by their content hash. Each package is immutable and
//! can be hard-linked into multiple state directories.

mod archive;
mod package;

pub use archive::{create_package, extract_package};
pub use package::StoredPackage;

use spsv2_errors::{Error, StorageError};
use spsv2_hash::{content_path, Hash};
use spsv2_root::{create_dir_all, exists, remove_dir_all, set_compression, size};
use std::path::{Path, PathBuf};

/// Store manager for content-addressed packages
#[derive(Clone)]
pub struct PackageStore {
    base_path: PathBuf,
}

impl PackageStore {
    /// Create a new store instance
    #[must_use]
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    /// Get the path for a package hash
    #[must_use]
    pub fn package_path(&self, hash: &Hash) -> PathBuf {
        let content = content_path(hash);
        self.base_path.join(content)
    }

    /// Check if a package exists in the store
    pub async fn has_package(&self, hash: &Hash) -> bool {
        let path = self.package_path(hash);
        exists(&path).await
    }

    /// Add a package to the store from a .sp file
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - File I/O operations fail
    /// - Package extraction fails
    /// - Package hash computation fails
    /// - Directory creation fails
    pub async fn add_package(&self, sp_file: &Path) -> Result<StoredPackage, Error> {
        // Compute hash of the .sp file
        let hash = Hash::hash_file(sp_file).await?;

        // Check if already exists
        let dest_path = self.package_path(&hash);
        if exists(&dest_path).await {
            return StoredPackage::load(&dest_path).await;
        }

        // Create parent directory
        if let Some(parent) = dest_path.parent() {
            create_dir_all(parent).await?;
        }

        // Extract to temporary directory
        let temp_dir = tempfile::tempdir().map_err(|e| StorageError::IoError {
            message: e.to_string(),
        })?;

        extract_package(sp_file, temp_dir.path()).await?;

        // Move to final location
        tokio::fs::rename(temp_dir.path(), &dest_path)
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to move package to store: {e}"),
            })?;

        // Set compression on macOS
        set_compression(&dest_path)?;

        StoredPackage::load(&dest_path).await
    }

    /// Remove a package from the store
    ///
    /// # Errors
    ///
    /// Returns an error if directory removal fails
    pub async fn remove_package(&self, hash: &Hash) -> Result<(), Error> {
        let path = self.package_path(hash);
        if exists(&path).await {
            remove_dir_all(&path).await?;
        }
        Ok(())
    }

    /// Get the size of a stored package
    ///
    /// # Errors
    ///
    /// Returns an error if the package path doesn't exist or size calculation fails
    pub async fn package_size(&self, hash: &Hash) -> Result<u64, Error> {
        let path = self.package_path(hash);
        size(&path).await
    }

    /// Link package contents into a destination
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Package loading fails
    /// - Linking operation fails
    pub async fn link_package(&self, hash: &Hash, dest_root: &Path) -> Result<(), Error> {
        let pkg = StoredPackage::load(&self.package_path(hash)).await?;
        pkg.link_to(dest_root).await
    }

    /// Get SBOM data for a package
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Package cannot be found by name/version
    /// - SBOM file cannot be read
    pub async fn get_package_sbom(
        &self,
        package_name: &str,
        package_version: &spsv2_types::Version,
    ) -> Result<Vec<u8>, Error> {
        // Get the package path (this is a simplified implementation)
        let package_path = self.get_package_path(package_name, package_version)?;

        // Try to read SPDX SBOM first
        let spdx_path = package_path.join("sbom.spdx.json");
        if exists(&spdx_path).await {
            return tokio::fs::read(&spdx_path).await.map_err(|e| {
                StorageError::IoError {
                    message: format!("failed to read SBOM file: {e}"),
                }
                .into()
            });
        }

        // Fall back to CycloneDX SBOM
        let cdx_path = package_path.join("sbom.cdx.json");
        if exists(&cdx_path).await {
            return tokio::fs::read(&cdx_path).await.map_err(|e| {
                StorageError::IoError {
                    message: format!("failed to read SBOM file: {e}"),
                }
                .into()
            });
        }

        // No SBOM found
        Err(StorageError::IoError {
            message: format!("SBOM file not found for package {package_name}-{package_version}"),
        }
        .into())
    }

    /// Get package path by name and version
    ///
    /// # Errors
    ///
    /// Currently returns a dummy implementation, but may return errors in future
    /// when actual package lookup is implemented
    pub fn get_package_path(
        &self,
        package_name: &str,
        package_version: &spsv2_types::Version,
    ) -> Result<std::path::PathBuf, Error> {
        // This is a simplified implementation - in reality we'd need to
        // look up the package hash from name/version
        // For now, create a dummy hash from name and version
        let dummy_content = format!("{package_name}-{package_version}");
        let hash = spsv2_hash::Hash::from_data(dummy_content.as_bytes());
        Ok(self.package_path(&hash))
    }

    /// Add a local package file to the store
    ///
    /// # Errors
    ///
    /// Returns an error if package addition fails
    pub async fn add_local_package(
        &self,
        local_path: &std::path::Path,
    ) -> Result<std::path::PathBuf, Error> {
        let stored_package = self.add_package(local_path).await?;
        Ok(stored_package.path().to_path_buf())
    }

    /// List all packages in the store
    ///
    /// # Errors
    ///
    /// Returns an error if directory traversal fails or I/O operations fail
    pub async fn list_packages(&self) -> Result<Vec<Hash>, Error> {
        let mut packages = Vec::new();

        let mut entries = tokio::fs::read_dir(&self.base_path).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }

            let name = entry.file_name();
            if let Some(prefix) = name.to_str() {
                // First level is 2-char prefix
                if prefix.len() != 2 {
                    continue;
                }

                let mut sub_entries = tokio::fs::read_dir(entry.path()).await?;
                while let Some(sub_entry) = sub_entries.next_entry().await? {
                    if !sub_entry.file_type().await?.is_dir() {
                        continue;
                    }

                    if let Some(suffix) = sub_entry.file_name().to_str() {
                        // Reconstruct full hash
                        let full_hash = format!("{prefix}{suffix}");
                        if let Ok(hash) = Hash::from_hex(&full_hash) {
                            packages.push(hash);
                        }
                    }
                }
            }
        }

        Ok(packages)
    }

    /// Clean up the store (remove empty directories)
    ///
    /// # Errors
    ///
    /// Returns an error if directory cleanup operations fail
    pub async fn cleanup(&self) -> Result<(), Error> {
        // Walk the store and remove empty directories
        self.cleanup_dir(&self.base_path).await?;
        Ok(())
    }

    async fn cleanup_dir(&self, dir: &Path) -> Result<bool, Error> {
        let mut is_empty = true;
        let mut entries = tokio::fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if entry.file_type().await?.is_dir() {
                if Box::pin(self.cleanup_dir(&path)).await? {
                    // Remove empty directory
                    let _ = tokio::fs::remove_dir(&path).await;
                } else {
                    is_empty = false;
                }
            } else {
                is_empty = false;
            }
        }

        Ok(is_empty)
    }

    /// Verify store integrity
    ///
    /// # Errors
    ///
    /// Returns an error if package listing fails or verification operations fail
    pub async fn verify(&self) -> Result<Vec<(Hash, String)>, Error> {
        let mut errors = Vec::new();

        for hash in self.list_packages().await? {
            let path = self.package_path(&hash);

            // Check manifest exists
            let manifest_path = path.join("manifest.toml");
            if !exists(&manifest_path).await {
                errors.push((hash, "missing manifest.toml".to_string()));
            }

            // Could add more verification here (file checksums, etc.)
        }

        Ok(errors)
    }

    /// Garbage collect unreferenced packages
    ///
    /// # Errors
    ///
    /// Currently returns success, but may return errors in future implementations
    /// when state manager integration is added
    pub fn garbage_collect(&self) -> Result<usize, Error> {
        // This would need to integrate with state manager to find unreferenced packages
        // For now, return 0 packages removed
        Ok(0)
    }

    /// Verify store integrity
    ///
    /// # Errors
    ///
    /// Returns an error if the base path doesn't exist or is not accessible
    pub fn verify_integrity(&self) -> Result<(), Error> {
        // Basic verification - check if base path exists and is accessible
        if !self.base_path.exists() {
            return Err(spsv2_errors::StorageError::DirectoryNotFound {
                path: self.base_path.clone(),
            }
            .into());
        }
        Ok(())
    }

    /// Get package size by name and version
    ///
    /// # Errors
    ///
    /// Currently returns success, but may return errors in future implementations
    /// when package lookup is implemented
    pub fn get_package_size(
        &self,
        _package_name: &str,
        _package_version: &spsv2_types::Version,
    ) -> Result<u64, Error> {
        // TODO: Implement lookup by package name/version
        // For now, return 0 as placeholder
        Ok(0)
    }

    /// Add package from file with specific name and version
    ///
    /// # Errors
    ///
    /// Returns an error if package addition fails
    pub async fn add_package_from_file(
        &self,
        file_path: &std::path::Path,
        _package_name: &str,
        _package_version: &spsv2_types::Version,
    ) -> Result<StoredPackage, Error> {
        // For now, just delegate to add_package
        self.add_package(file_path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_store_operations() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        // Test with a dummy hash
        let hash = Hash::from_data(b"test package");

        // Initially shouldn't exist
        assert!(!store.has_package(&hash).await);

        // Test package path generation
        let path = store.package_path(&hash);
        assert!(path.starts_with(temp.path()));
        assert!(path.to_str().unwrap().contains(&hash.to_hex()[..2]));
    }

    #[tokio::test]
    async fn test_list_packages() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        // Create some fake package directories
        let hash1 = Hash::from_data(b"package1");
        let path1 = store.package_path(&hash1);
        create_dir_all(&path1).await.unwrap();

        let hash2 = Hash::from_data(b"package2");
        let path2 = store.package_path(&hash2);
        create_dir_all(&path2).await.unwrap();

        // List should find both
        let packages = store.list_packages().await.unwrap();
        assert_eq!(packages.len(), 2);
        assert!(packages.contains(&hash1));
        assert!(packages.contains(&hash2));
    }
}

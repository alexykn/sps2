#![warn(mismatched_lifetime_syntaxes)]
#![deny(clippy::pedantic, unsafe_code)]

//! Content-addressed storage for sps2
//!
//! This crate manages the `/opt/pm/store/` directory where packages
//! are stored by their content hash. Each package is immutable and
//! can be hard-linked into multiple state directories.

mod archive;
mod file_store;
mod format_detection;
pub mod manifest_io;
mod package;

pub use archive::{
    create_package, extract_package, extract_package_with_events, list_package_contents,
};
pub use file_store::{FileStore, FileVerificationResult};
pub use format_detection::{PackageFormatDetector, PackageFormatInfo, StoreFormatValidator};
pub use package::StoredPackage;

use sps2_errors::{Error, StorageError};
use sps2_hash::Hash;
use sps2_platform::filesystem_helpers::set_compression;
use sps2_platform::PlatformManager;
use std::path::{Path, PathBuf};

/// Store manager for content-addressed packages
#[derive(Clone, Debug)]
pub struct PackageStore {
    base_path: PathBuf,
    format_validator: StoreFormatValidator,
    file_store: FileStore,
}

impl PackageStore {
    /// Create a new store instance
    #[must_use]
    pub fn new(base_path: PathBuf) -> Self {
        let file_store = FileStore::new(&base_path);
        Self {
            base_path,
            format_validator: StoreFormatValidator::new(),
            file_store,
        }
    }

    /// Create a new store instance that allows incompatible package formats
    ///
    /// This is useful for migration tools that need to work with older package formats
    #[must_use]
    pub fn new_with_migration_support(base_path: PathBuf) -> Self {
        let file_store = FileStore::new(&base_path);
        Self {
            base_path,
            format_validator: StoreFormatValidator::allow_incompatible(),
            file_store,
        }
    }

    /// Create a platform context for filesystem operations
    fn create_platform_context() -> (
        &'static sps2_platform::Platform,
        sps2_platform::core::PlatformContext,
    ) {
        let platform = PlatformManager::instance().platform();
        let context = platform.create_context(None);
        (platform, context)
    }

    /// Get the path for a package hash
    #[must_use]
    pub fn package_path(&self, hash: &Hash) -> PathBuf {
        self.base_path.join("packages").join(hash.to_hex())
    }

    /// Get the file store for file-level operations
    #[must_use]
    pub fn file_store(&self) -> &FileStore {
        &self.file_store
    }

    /// Get the path to a file in the file store by its hash
    #[must_use]
    pub fn file_path(&self, hash: &Hash) -> PathBuf {
        self.file_store.file_path(hash)
    }

    /// Check if a package exists in the store
    pub async fn has_package(&self, hash: &Hash) -> bool {
        let path = self.package_path(hash);
        let (platform, ctx) = Self::create_platform_context();
        platform.filesystem().exists(&ctx, &path).await
    }

    /// Load a stored package if it exists in the store
    ///
    /// # Errors
    ///
    /// Returns an error if the package metadata cannot be loaded from disk.
    pub async fn load_package_if_exists(
        &self,
        hash: &Hash,
    ) -> Result<Option<StoredPackage>, Error> {
        if self.has_package(hash).await {
            let package_path = self.package_path(hash);
            let package = StoredPackage::load(&package_path).await?;
            Ok(Some(package))
        } else {
            Ok(None)
        }
    }

    /// Add a package to the store from a .sp file
    ///
    /// This extracts the package, hashes individual files for deduplication,
    /// and stores the package metadata with file references.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - File I/O operations fail
    /// - Package extraction fails
    /// - Package hash computation fails
    /// - Directory creation fails
    /// - Package format is incompatible
    pub async fn add_package(&self, sp_file: &Path) -> Result<StoredPackage, Error> {
        // Validate package format before processing (no direct printing here)
        self.format_validator
            .validate_before_storage(sp_file)
            .await?;

        // Extract to temporary directory first
        let temp_dir = tempfile::tempdir().map_err(|e| StorageError::IoError {
            message: e.to_string(),
        })?;

        extract_package(sp_file, temp_dir.path()).await?;

        // Compute hash of the extracted contents for package identity
        let package_hash = sps2_hash::Hash::hash_directory(temp_dir.path()).await?;

        // Check if package already exists
        let package_path = self.base_path.join("packages").join(package_hash.to_hex());
        let (platform, ctx) = Self::create_platform_context();

        if platform.filesystem().exists(&ctx, &package_path).await {
            // Package already stored, just return it
            return StoredPackage::load(&package_path).await;
        }

        // Initialize file store if needed
        self.file_store.initialize().await?;

        // Hash and store all individual files
        let file_results = self.file_store.store_directory(temp_dir.path()).await?;

        // Create package directory
        platform
            .filesystem()
            .create_dir_all(&ctx, &package_path)
            .await?;

        // Copy manifest to package directory
        let manifest_src = temp_dir.path().join("manifest.toml");
        let manifest_dest = package_path.join("manifest.toml");
        tokio::fs::copy(&manifest_src, &manifest_dest).await?;

        // Copy SBOM if it exists
        for sbom_name in &["sbom.spdx.json", "sbom.cdx.json"] {
            let sbom_src = temp_dir.path().join(sbom_name);
            if platform.filesystem().exists(&ctx, &sbom_src).await {
                let sbom_dest = package_path.join(sbom_name);
                tokio::fs::copy(&sbom_src, &sbom_dest).await?;
            }
        }

        // Create files.json with all file references
        let files_json =
            serde_json::to_string_pretty(&file_results).map_err(|e| StorageError::IoError {
                message: format!("failed to serialize file results: {e}"),
            })?;
        let files_path = package_path.join("files.json");
        tokio::fs::write(&files_path, files_json).await?;

        // Set compression on macOS
        set_compression(&package_path)?;

        StoredPackage::load(&package_path).await
    }

    /// Remove a package from the store
    ///
    /// # Errors
    ///
    /// Returns an error if directory removal fails
    pub async fn remove_package(&self, hash: &Hash) -> Result<(), Error> {
        let path = self.package_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        if platform.filesystem().exists(&ctx, &path).await {
            platform.filesystem().remove_dir_all(&ctx, &path).await?;
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
        let (platform, ctx) = Self::create_platform_context();
        platform.filesystem().size(&ctx, &path).await.map_err(|e| {
            StorageError::IoError {
                message: e.to_string(),
            }
            .into()
        })
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
    /// - Package cannot be found by hash
    /// - SBOM file cannot be read
    pub async fn get_package_sbom(&self, hash: &Hash) -> Result<Vec<u8>, Error> {
        // Get the package path using the hash
        let package_path = self.package_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        // Try to read SPDX SBOM first
        let spdx_path = package_path.join("sbom.spdx.json");
        if platform.filesystem().exists(&ctx, &spdx_path).await {
            return tokio::fs::read(&spdx_path).await.map_err(|e| {
                StorageError::IoError {
                    message: format!("failed to read SBOM file: {e}"),
                }
                .into()
            });
        }

        // Fall back to CycloneDX SBOM
        let cdx_path = package_path.join("sbom.cdx.json");
        if platform.filesystem().exists(&ctx, &cdx_path).await {
            return tokio::fs::read(&cdx_path).await.map_err(|e| {
                StorageError::IoError {
                    message: format!("failed to read SBOM file: {e}"),
                }
                .into()
            });
        }

        // No SBOM found
        Err(StorageError::IoError {
            message: format!(
                "SBOM file not found for package with hash {}",
                hash.to_hex()
            ),
        }
        .into())
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
            if let Some(hash_str) = name.to_str() {
                // Each directory name should be a complete hash (flat structure)
                if let Ok(hash) = Hash::from_hex(hash_str) {
                    packages.push(hash);
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

        let (platform, ctx) = Self::create_platform_context();

        for hash in self.list_packages().await? {
            let path = self.package_path(&hash);

            // Check manifest exists
            let manifest_path = path.join("manifest.toml");
            if !platform.filesystem().exists(&ctx, &manifest_path).await {
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
            return Err(sps2_errors::StorageError::DirectoryNotFound {
                path: self.base_path.clone(),
            }
            .into());
        }
        Ok(())
    }

    /// Get package format information from a .sp file
    ///
    /// # Errors
    ///
    /// Returns an error if format detection fails
    pub async fn get_package_format_info(
        &self,
        sp_file: &Path,
    ) -> Result<PackageFormatInfo, Error> {
        let detector = PackageFormatDetector::new();
        detector.detect_format(sp_file).await
    }

    /// Get package format information from a stored package
    ///
    /// # Errors
    ///
    /// Returns an error if the package is not found or format detection fails
    pub async fn get_stored_package_format_info(
        &self,
        hash: &Hash,
    ) -> Result<PackageFormatInfo, Error> {
        let package_path = self.package_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        if !platform.filesystem().exists(&ctx, &package_path).await {
            return Err(StorageError::PackageNotFound {
                hash: hash.to_hex(),
            }
            .into());
        }

        self.format_validator
            .validate_stored_package(&package_path)
            .await
    }

    /// Check if a stored package is compatible with the current format version
    ///
    /// # Errors
    ///
    /// Returns an error if the package is not found or format detection fails
    pub async fn is_package_compatible(&self, hash: &Hash) -> Result<bool, Error> {
        let format_info = self.get_stored_package_format_info(hash).await?;
        Ok(format_info.is_compatible)
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
        _package_version: &sps2_types::Version,
    ) -> Result<StoredPackage, Error> {
        // For now, just delegate to add_package
        self.add_package(file_path).await
    }

    /// Add package from staging directory
    ///
    /// This computes the package hash, stores individual files,
    /// and creates the package metadata directory.
    ///
    /// # Errors
    ///
    /// Returns an error if staging directory processing fails
    pub async fn add_package_from_staging(
        &self,
        staging_path: &std::path::Path,
        _package_id: &sps2_resolver::PackageId,
    ) -> Result<StoredPackage, Error> {
        // Read manifest to get package info
        let manifest_path = staging_path.join("manifest.toml");
        let _manifest_content = tokio::fs::read_to_string(&manifest_path)
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to read manifest from staging: {e}"),
            })?;

        // Compute hash of staging directory contents for package identity
        let package_hash = self.compute_staging_hash(staging_path).await?;

        // Check if package already exists
        let package_path = self.base_path.join("packages").join(package_hash.to_hex());
        let (platform, ctx) = Self::create_platform_context();

        if platform.filesystem().exists(&ctx, &package_path).await {
            return StoredPackage::load(&package_path).await;
        }

        // Initialize file store if needed
        self.file_store.initialize().await?;

        // Hash and store all individual files
        let file_results = self.file_store.store_directory(staging_path).await?;

        // Create package directory
        platform
            .filesystem()
            .create_dir_all(&ctx, &package_path)
            .await?;

        // Copy manifest to package directory
        let manifest_dest = package_path.join("manifest.toml");
        tokio::fs::copy(&manifest_path, &manifest_dest).await?;

        // Copy SBOM if it exists
        for sbom_name in &["sbom.spdx.json", "sbom.cdx.json"] {
            let sbom_src = staging_path.join(sbom_name);
            if platform.filesystem().exists(&ctx, &sbom_src).await {
                let sbom_dest = package_path.join(sbom_name);
                tokio::fs::copy(&sbom_src, &sbom_dest).await?;
            }
        }

        // Create files.json with all file references
        let files_json =
            serde_json::to_string_pretty(&file_results).map_err(|e| StorageError::IoError {
                message: format!("failed to serialize file results: {e}"),
            })?;
        let files_path = package_path.join("files.json");
        tokio::fs::write(&files_path, files_json).await?;

        // Set compression on macOS
        set_compression(&package_path)?;

        StoredPackage::load(&package_path).await
    }

    /// Compute hash of staging directory contents
    async fn compute_staging_hash(&self, staging_path: &Path) -> Result<Hash, Error> {
        // Hash the entire staging directory for true content-addressable storage
        sps2_hash::Hash::hash_directory(staging_path).await
    }
}

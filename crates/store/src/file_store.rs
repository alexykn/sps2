//! File-level content-addressed storage operations
//!
//! This module provides functionality for storing individual files
//! by their content hash, enabling deduplication across packages.

use sps2_errors::{Error, StorageError};
use sps2_hash::{calculate_file_storage_path, FileHashResult, FileHasher, FileHasherConfig, Hash};
use sps2_platform::core::PlatformContext;
use sps2_platform::PlatformManager;
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

/// Result of file verification operation
#[derive(Debug, Clone, PartialEq)]
pub enum FileVerificationResult {
    /// File is valid and matches expected hash
    Valid,
    /// File is missing from store
    Missing,
    /// File exists but hash doesn't match
    HashMismatch { expected: Hash, actual: Hash },
    /// Verification failed due to error
    Error { message: String },
}

/// File store for content-addressed file storage
#[derive(Clone, Debug)]
pub struct FileStore {
    /// Base path for file objects (/opt/pm/store/objects)
    objects_path: PathBuf,
    /// File hasher for computing file hashes
    file_hasher: FileHasher,
}

impl FileStore {
    /// Create a new file store instance
    #[must_use]
    pub fn new(store_base_path: &Path) -> Self {
        let objects_path = store_base_path.join("objects");
        let file_hasher = FileHasher::new(FileHasherConfig::default());

        Self {
            objects_path,
            file_hasher,
        }
    }

    /// Create a platform context for filesystem operations
    fn create_platform_context() -> (&'static sps2_platform::Platform, PlatformContext) {
        let platform = PlatformManager::instance().platform();
        let context = platform.create_context(None);
        (platform, context)
    }

    /// Initialize the file store directory structure
    ///
    /// # Errors
    /// Returns an error if directory creation fails
    pub async fn initialize(&self) -> Result<(), Error> {
        // Create the objects directory
        let (platform, ctx) = Self::create_platform_context();
        platform
            .filesystem()
            .create_dir_all(&ctx, &self.objects_path)
            .await?;

        // Create prefix directories (00-ff)
        for i in 0..256 {
            let prefix1 = format!("{i:02x}");
            let prefix1_path = self.objects_path.join(&prefix1);
            platform
                .filesystem()
                .create_dir_all(&ctx, &prefix1_path)
                .await?;
            for j in 0..256 {
                let prefix2 = format!("{j:02x}");
                let prefix2_path = prefix1_path.join(&prefix2);
                platform
                    .filesystem()
                    .create_dir_all(&ctx, &prefix2_path)
                    .await?;
            }
        }

        Ok(())
    }

    /// Get the storage path for a file hash
    #[must_use]
    pub fn file_path(&self, hash: &Hash) -> PathBuf {
        let (prefix, full_hash) = calculate_file_storage_path(hash);
        self.objects_path.join(prefix).join(full_hash)
    }

    /// Check if a file exists in the store
    pub async fn has_file(&self, hash: &Hash) -> bool {
        let path = self.file_path(hash);
        let (platform, ctx) = Self::create_platform_context();
        platform.filesystem().exists(&ctx, &path).await
    }

    /// Store a file by its content hash
    ///
    /// Returns true if the file was newly stored, false if it already existed
    ///
    /// # Errors
    /// Returns an error if file operations fail
    pub async fn store_file(&self, source_path: &Path, hash: &Hash) -> Result<bool, Error> {
        let dest_path = self.file_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        if platform.filesystem().exists(&ctx, &dest_path).await {
            return Ok(false);
        }

        // Ensure parent directory exists
        let parent_dir = dest_path.parent().ok_or_else(|| StorageError::IoError {
            message: "failed to get parent directory".to_string(),
        })?;
        platform
            .filesystem()
            .create_dir_all(&ctx, parent_dir)
            .await?;

        // Create a unique temporary file path
        let temp_file_name = format!("{}.tmp", Uuid::new_v4());
        let temp_path = parent_dir.join(temp_file_name);

        // Copy to temporary file
        if let Err(e) = fs::copy(source_path, &temp_path).await {
            // Clean up temp file on failure
            let _ = platform.filesystem().remove_file(&ctx, &temp_path).await;
            return Err(StorageError::IoError {
                message: format!("failed to copy file to temp: {e}"),
            }
            .into());
        }

        // Attempt to atomically rename (move) the file
        match fs::rename(&temp_path, &dest_path).await {
            Ok(()) => {
                // Make file read-only after successful move
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let metadata = fs::metadata(&dest_path).await?;
                    let mut perms = metadata.permissions();
                    let mode = perms.mode() & 0o555; // Remove write permissions
                    perms.set_mode(mode);
                    fs::set_permissions(&dest_path, perms).await?;
                }
                Ok(true)
            }
            Err(e) => {
                // Clean up the temporary file
                let _ = platform.filesystem().remove_file(&ctx, &temp_path).await;

                // If the error is because the file exists, another process/thread beat us to it.
                // This is not an error condition for us.
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(false)
                } else {
                    Err(StorageError::IoError {
                        message: format!("failed to move temp file to store: {e}"),
                    }
                    .into())
                }
            }
        }
    }

    /// Store a file and compute its hash
    ///
    /// # Errors
    /// Returns an error if file operations fail
    pub async fn store_file_with_hash(&self, source_path: &Path) -> Result<(Hash, bool), Error> {
        // Compute hash
        let hash = Hash::hash_file(source_path).await?;

        // Store file
        let newly_stored = self.store_file(source_path, &hash).await?;

        Ok((hash, newly_stored))
    }

    /// Link a stored file to a destination
    ///
    /// # Errors
    /// Returns an error if the file doesn't exist or linking fails
    pub async fn link_file(&self, hash: &Hash, dest_path: &Path) -> Result<(), Error> {
        let source_path = self.file_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        if !platform.filesystem().exists(&ctx, &source_path).await {
            return Err(StorageError::PathNotFound {
                path: source_path.display().to_string(),
            }
            .into());
        }

        // Ensure parent directory exists
        if let Some(parent) = dest_path.parent() {
            platform.filesystem().create_dir_all(&ctx, parent).await?;
        }

        // Remove existing file if it exists
        if platform.filesystem().exists(&ctx, dest_path).await {
            platform.filesystem().remove_file(&ctx, dest_path).await?;
        }

        // Use APFS clonefile on macOS for copy-on-write semantics
        // This prevents corruption of the store when files are modified in place
        platform
            .filesystem()
            .clone_file(&ctx, &source_path, dest_path)
            .await?;

        Ok(())
    }

    /// Store all files from a directory
    ///
    /// Returns a list of file hash results
    ///
    /// # Errors
    /// Returns an error if directory traversal or file operations fail
    pub async fn store_directory(&self, dir_path: &Path) -> Result<Vec<FileHashResult>, Error> {
        // Hash all files in the directory
        let hash_results = self.file_hasher.hash_directory(dir_path).await?;

        // Filter out manifest.toml and sbom files before storing
        // Also fix paths by stripping opt/pm/live/ prefix
        let mut filtered_results = Vec::new();

        for result in hash_results {
            // Skip manifest and sbom files - they should only exist in package metadata
            if result.relative_path == "manifest.toml"
                || result.relative_path == "sbom.spdx.json"
                || result.relative_path == "sbom.cdx.json"
            {
                continue;
            }

            // Skip opt/pm/live directory entries themselves
            if result.relative_path == "opt"
                || result.relative_path == "opt/pm"
                || result.relative_path == "opt/pm/live"
            {
                continue;
            }

            // Store the file if it's not a directory or symlink
            if !result.is_directory && !result.is_symlink {
                // Use original path for file storage
                let original_path = result.relative_path.clone();
                let file_path = dir_path.join(&original_path);
                self.store_file(&file_path, &result.hash).await?;
            }

            filtered_results.push(result);
        }

        Ok(filtered_results)
    }

    /// Link files from hash results to a destination directory
    ///
    /// # Errors
    /// Returns an error if linking operations fail
    pub async fn link_files(
        &self,
        hash_results: &[FileHashResult],
        source_base: &Path,
        dest_base: &Path,
    ) -> Result<(), Error> {
        let (platform, ctx) = Self::create_platform_context();

        for result in hash_results {
            // Skip manifest.toml and sbom files - they should only exist in store
            if result.relative_path == "manifest.toml"
                || result.relative_path == "sbom.spdx.json"
                || result.relative_path == "sbom.cdx.json"
            {
                continue;
            }

            let dest_path = dest_base.join(&result.relative_path);

            if result.is_directory {
                // Create directory
                platform
                    .filesystem()
                    .create_dir_all(&ctx, &dest_path)
                    .await?;
            } else if result.is_symlink {
                // Recreate symlink
                let source_path = source_base.join(&result.relative_path);
                if let Ok(target) = fs::read_link(&source_path).await {
                    // Ensure parent directory exists
                    if let Some(parent) = dest_path.parent() {
                        platform.filesystem().create_dir_all(&ctx, parent).await?;
                    }

                    // Remove existing symlink if it exists
                    if platform.filesystem().exists(&ctx, &dest_path).await {
                        platform.filesystem().remove_file(&ctx, &dest_path).await?;
                    }

                    // Create symlink
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::symlink;
                        symlink(&target, &dest_path)?;
                    }
                }
            } else {
                // Link regular file
                self.link_file(&result.hash, &dest_path).await?;
            }
        }

        Ok(())
    }

    /// Remove a file from the store
    ///
    /// # Errors
    /// Returns an error if file removal fails
    pub async fn remove_file(&self, hash: &Hash) -> Result<(), Error> {
        let path = self.file_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        if platform.filesystem().exists(&ctx, &path).await {
            platform.filesystem().remove_file(&ctx, &path).await?;
        }
        Ok(())
    }

    /// Get the size of a stored file
    ///
    /// # Errors
    /// Returns an error if the file doesn't exist or metadata cannot be read
    pub async fn file_size(&self, hash: &Hash) -> Result<u64, Error> {
        let path = self.file_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        platform.filesystem().size(&ctx, &path).await.map_err(|_| {
            StorageError::PathNotFound {
                path: path.display().to_string(),
            }
            .into()
        })
    }

    /// Verify that a stored file matches its expected hash
    ///
    /// # Errors
    /// Returns an error if the file doesn't exist or hashing fails
    pub async fn verify_file(&self, hash: &Hash) -> Result<bool, Error> {
        let path = self.file_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        if !platform.filesystem().exists(&ctx, &path).await {
            return Ok(false);
        }

        // Use the same algorithm as the expected hash for verification
        let actual_hash = Hash::hash_file_with_algorithm(&path, hash.algorithm()).await?;
        Ok(actual_hash == *hash)
    }

    /// Verify a stored file and return detailed result
    ///
    /// This function performs verification and returns a detailed result
    /// that can be used by higher-level verification systems.
    ///
    /// # Errors
    /// Returns an error if verification fails due to I/O issues
    pub async fn verify_file_detailed(&self, hash: &Hash) -> Result<FileVerificationResult, Error> {
        let path = self.file_path(hash);
        let (platform, ctx) = Self::create_platform_context();

        if !platform.filesystem().exists(&ctx, &path).await {
            return Ok(FileVerificationResult::Missing);
        }

        // Use the same algorithm as the expected hash for verification
        match Hash::hash_file_with_algorithm(&path, hash.algorithm()).await {
            Ok(actual_hash) => {
                if actual_hash == *hash {
                    Ok(FileVerificationResult::Valid)
                } else {
                    Ok(FileVerificationResult::HashMismatch {
                        expected: hash.clone(),
                        actual: actual_hash,
                    })
                }
            }
            Err(e) => Ok(FileVerificationResult::Error {
                message: e.to_string(),
            }),
        }
    }

    /// Clean up empty prefix directories
    ///
    /// # Errors
    /// Returns an error if directory operations fail
    pub async fn cleanup(&self) -> Result<(), Error> {
        let mut entries = fs::read_dir(&self.objects_path).await?;

        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                let prefix_path = entry.path();

                // Check if directory is empty
                let mut prefix_entries = fs::read_dir(&prefix_path).await?;
                if prefix_entries.next_entry().await?.is_none() {
                    // Directory is empty, remove it
                    let _ = fs::remove_dir(&prefix_path).await;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_file_store_operations() {
        let temp_dir = TempDir::new().unwrap();
        let store = FileStore::new(temp_dir.path());

        // Initialize store
        store.initialize().await.unwrap();

        // Create a test file
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, b"Hello, world!").await.unwrap();

        // Store file
        let (hash, newly_stored) = store.store_file_with_hash(&test_file).await.unwrap();
        assert!(newly_stored);

        // Check file exists
        assert!(store.has_file(&hash).await);

        // Store same file again
        let (_, newly_stored) = store.store_file_with_hash(&test_file).await.unwrap();
        assert!(!newly_stored); // Should already exist

        // Link file to new location
        let link_dest = temp_dir.path().join("linked.txt");
        store.link_file(&hash, &link_dest).await.unwrap();

        // Verify linked file content
        let content = fs::read(&link_dest).await.unwrap();
        assert_eq!(content, b"Hello, world!");

        // Verify file integrity
        assert!(store.verify_file(&hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_directory_storage() {
        let temp_dir = TempDir::new().unwrap();
        let store = FileStore::new(temp_dir.path());
        store.initialize().await.unwrap();

        // Create test directory structure
        let test_dir = temp_dir.path().join("test_pkg");
        fs::create_dir(&test_dir).await.unwrap();
        fs::write(test_dir.join("file1.txt"), b"content1")
            .await
            .unwrap();
        fs::create_dir(test_dir.join("subdir")).await.unwrap();
        fs::write(test_dir.join("subdir/file2.txt"), b"content2")
            .await
            .unwrap();

        // Store directory
        let results = store.store_directory(&test_dir).await.unwrap();

        // Should have entries for files and directories
        assert!(results.len() >= 2); // At least the two files

        // Link files to new location
        let dest_dir = temp_dir.path().join("linked_pkg");
        store
            .link_files(&results, &test_dir, &dest_dir)
            .await
            .unwrap();

        // Verify linked files
        assert!(dest_dir.join("file1.txt").exists());
        assert!(dest_dir.join("subdir/file2.txt").exists());
    }
}

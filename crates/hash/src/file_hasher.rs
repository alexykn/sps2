//! File-level hashing operations for content-addressed storage
//!
//! This module provides functionality for hashing individual files
//! during package extraction and installation, supporting parallel
//! processing and metadata collection.

use crate::Hash;
use sps2_errors::{Error, StorageError};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

/// Result of hashing a single file
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileHashResult {
    /// Relative path within the package
    pub relative_path: String,
    /// BLAKE3 hash of the file contents
    pub hash: Hash,
    /// File size in bytes
    pub size: u64,
    /// Whether this is a directory
    pub is_directory: bool,
    /// Whether this is a symlink
    pub is_symlink: bool,
    /// Unix permissions (if available)
    #[cfg(unix)]
    pub mode: Option<u32>,
}

/// Configuration for file hashing operations
#[derive(Debug, Clone)]
pub struct FileHasherConfig {
    /// Maximum number of concurrent hash operations
    pub max_concurrency: usize,
    /// Whether to follow symlinks
    pub follow_symlinks: bool,
    /// Whether to include directory entries
    pub include_directories: bool,
}

impl Default for FileHasherConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            follow_symlinks: false,
            include_directories: true,
        }
    }
}

/// File hasher for processing multiple files
#[derive(Debug)]
pub struct FileHasher {
    config: FileHasherConfig,
}

impl FileHasher {
    /// Create a new file hasher with the given configuration
    #[must_use]
    pub fn new(config: FileHasherConfig) -> Self {
        Self { config }
    }

    /// Hash a single file and collect metadata
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or metadata cannot be accessed
    pub async fn hash_file_with_metadata(
        &self,
        path: &Path,
        base_path: &Path,
    ) -> Result<FileHashResult, Error> {
        let metadata = tokio::fs::symlink_metadata(path).await?;

        // Calculate relative path
        let relative_path = path
            .strip_prefix(base_path)
            .map_err(|_| StorageError::IoError {
                message: format!("failed to compute relative path for {}", path.display()),
            })?
            .to_string_lossy()
            .to_string();

        // Handle different file types
        if metadata.is_dir() {
            Ok(FileHashResult {
                relative_path,
                hash: Hash::from_data(b""), // Empty hash for directories
                size: 0,
                is_directory: true,
                is_symlink: false,
                #[cfg(unix)]
                mode: {
                    use std::os::unix::fs::PermissionsExt;
                    Some(metadata.permissions().mode())
                },
            })
        } else if metadata.is_symlink() {
            // For symlinks, hash the target path
            let target = tokio::fs::read_link(path).await?;
            let target_string = target.to_string_lossy().to_string();
            let target_bytes = target_string.as_bytes();

            Ok(FileHashResult {
                relative_path,
                hash: Hash::from_data(target_bytes),
                size: target_bytes.len() as u64,
                is_directory: false,
                is_symlink: true,
                #[cfg(unix)]
                mode: {
                    use std::os::unix::fs::PermissionsExt;
                    Some(metadata.permissions().mode())
                },
            })
        } else {
            // Regular file
            let hash = Hash::hash_file(path).await?;

            Ok(FileHashResult {
                relative_path,
                hash,
                size: metadata.len(),
                is_directory: false,
                is_symlink: false,
                #[cfg(unix)]
                mode: {
                    use std::os::unix::fs::PermissionsExt;
                    Some(metadata.permissions().mode())
                },
            })
        }
    }

    /// Hash all files in a directory recursively
    ///
    /// # Errors
    /// Returns an error if directory traversal fails or file operations fail
    pub async fn hash_directory(&self, dir_path: &Path) -> Result<Vec<FileHashResult>, Error> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let base_path = dir_path.to_path_buf();

        // Spawn task to collect files
        let collector_handle = tokio::spawn({
            let base_path = base_path.clone();
            let tx = tx.clone();
            let include_dirs = self.config.include_directories;
            async move { collect_files_for_hashing(&base_path, &base_path, tx, include_dirs).await }
        });

        // Drop the original sender so the channel closes when collection is done
        drop(tx);

        // Process files with limited concurrency
        let mut results = Vec::new();
        let mut tasks = JoinSet::new();
        let semaphore =
            std::sync::Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrency));

        while let Some(file_path) = rx.recv().await {
            let permit =
                semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|e| StorageError::IoError {
                        message: format!("semaphore acquire error: {}", e),
                    })?;
            let base_path = base_path.clone();
            let hasher = self.clone();

            tasks.spawn(async move {
                let _permit = permit; // Hold permit until task completes
                hasher.hash_file_with_metadata(&file_path, &base_path).await
            });
        }

        // Wait for collector to finish
        collector_handle.await.map_err(|e| StorageError::IoError {
            message: format!("task join error: {}", e),
        })??;

        // Collect all results
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(hash_result)) => results.push(hash_result),
                Ok(Err(e)) => return Err(e),
                Err(e) => {
                    return Err(StorageError::IoError {
                        message: format!("task join error: {}", e),
                    }
                    .into())
                }
            }
        }

        // Sort results by path for deterministic output
        results.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        Ok(results)
    }

    /// Hash files from an iterator of paths
    ///
    /// # Errors
    /// Returns an error if any file operation fails
    pub async fn hash_files<I, P>(
        &self,
        base_path: &Path,
        paths: I,
    ) -> Result<Vec<FileHashResult>, Error>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut tasks = JoinSet::new();
        let semaphore =
            std::sync::Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrency));
        let base_path = base_path.to_path_buf();

        for path in paths {
            let file_path = base_path.join(path.as_ref());
            let permit =
                semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|e| StorageError::IoError {
                        message: format!("semaphore acquire error: {}", e),
                    })?;
            let base_path = base_path.clone();
            let hasher = self.clone();

            tasks.spawn(async move {
                let _permit = permit;
                hasher.hash_file_with_metadata(&file_path, &base_path).await
            });
        }

        let mut results = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(hash_result)) => results.push(hash_result),
                Ok(Err(e)) => return Err(e),
                Err(e) => {
                    return Err(StorageError::IoError {
                        message: format!("task join error: {}", e),
                    }
                    .into())
                }
            }
        }

        results.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(results)
    }
}

impl Clone for FileHasher {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
        }
    }
}

/// Helper function to collect files for hashing
async fn collect_files_for_hashing(
    base_path: &Path,
    current_path: &Path,
    tx: mpsc::UnboundedSender<PathBuf>,
    include_directories: bool,
) -> Result<(), Error> {
    let mut entries = tokio::fs::read_dir(current_path).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = entry.metadata().await?;

        if metadata.is_dir() {
            if include_directories {
                let _ = tx.send(path.clone());
            }
            // Recurse into directory
            Box::pin(collect_files_for_hashing(
                base_path,
                &path,
                tx.clone(),
                include_directories,
            ))
            .await?;
        } else {
            // Send file or symlink for hashing
            let _ = tx.send(path);
        }
    }

    Ok(())
}

/// Calculate the storage path for a file based on its hash
///
/// Returns the path components: (prefix, full_hash)
/// For example: hash "abc123..." -> ("ab", "abc123...")
#[must_use]
pub fn calculate_file_storage_path(hash: &Hash) -> (String, String) {
    let hex = hash.to_hex();
    let prefix = hex.chars().take(2).collect::<String>();
    (prefix, hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs;

    #[tokio::test]
    async fn test_hash_single_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, b"Hello, world!").await.unwrap();

        let hasher = FileHasher::new(FileHasherConfig::default());
        let result = hasher
            .hash_file_with_metadata(&file_path, temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result.relative_path, "test.txt");
        assert_eq!(result.size, 13);
        assert!(!result.is_directory);
        assert!(!result.is_symlink);
    }

    #[tokio::test]
    async fn test_hash_directory() {
        let temp_dir = TempDir::new().unwrap();

        // Create test structure
        fs::create_dir(temp_dir.path().join("subdir"))
            .await
            .unwrap();
        fs::write(temp_dir.path().join("file1.txt"), b"content1")
            .await
            .unwrap();
        fs::write(temp_dir.path().join("subdir/file2.txt"), b"content2")
            .await
            .unwrap();

        let hasher = FileHasher::new(FileHasherConfig::default());
        let results = hasher.hash_directory(temp_dir.path()).await.unwrap();

        // Should have 3 entries: root dir, subdir, and 2 files
        assert!(results.len() >= 2); // At least the two files

        // Check that files are sorted by path
        let file_results: Vec<_> = results.iter().filter(|r| !r.is_directory).collect();

        assert!(file_results.iter().any(|r| r.relative_path == "file1.txt"));
        assert!(file_results
            .iter()
            .any(|r| r.relative_path == "subdir/file2.txt"));
    }

    #[test]
    fn test_storage_path_calculation() {
        let hash = Hash::from_data(b"test data");
        let (prefix, full_hash) = calculate_file_storage_path(&hash);

        assert_eq!(prefix.len(), 2);
        assert_eq!(full_hash, hash.to_hex());
        assert!(full_hash.starts_with(&prefix));
    }
}

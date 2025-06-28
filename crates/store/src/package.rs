//! Stored package representation and operations

use sps2_errors::{Error, PackageError};
use sps2_hash::FileHashResult;
use sps2_manifest::Manifest;
use sps2_root::{create_dir_all, exists, hard_link, size};
use std::path::{Path, PathBuf};
use tokio::fs;

/// A package stored in the content-addressed store
pub struct StoredPackage {
    path: PathBuf,
    manifest: Manifest,
    /// File hash results if available (for new file-level packages)
    file_hashes: Option<Vec<FileHashResult>>,
}

impl StoredPackage {
    /// Load a stored package
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The manifest file cannot be found or read
    /// - The manifest file is invalid
    pub async fn load(path: &Path) -> Result<Self, Error> {
        let manifest_path = path.join("manifest.toml");
        let manifest = Manifest::from_file(&manifest_path).await?;

        // Try to load file hashes if available
        let files_json_path = path.join("files.json");
        let file_hashes = if exists(&files_json_path).await {
            let content = fs::read_to_string(&files_json_path).await?;
            serde_json::from_str(&content).ok()
        } else {
            None
        };

        Ok(Self {
            path: path.to_path_buf(),
            manifest,
            file_hashes,
        })
    }

    /// Get the package manifest
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Get the package path
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the package hash from the path
    #[must_use]
    pub fn hash(&self) -> Option<sps2_hash::Hash> {
        // The hash is the last component of the path
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|hash_str| sps2_hash::Hash::from_hex(hash_str).ok())
    }

    /// Check if this package has file-level hashes
    #[must_use]
    pub fn has_file_hashes(&self) -> bool {
        self.file_hashes.is_some()
    }

    /// Get the file hashes if available
    #[must_use]
    pub fn file_hashes(&self) -> Option<&[FileHashResult]> {
        self.file_hashes.as_deref()
    }

    /// Get the files directory
    #[must_use]
    pub fn files_path(&self) -> PathBuf {
        // New structure: files are under opt/pm/live
        let live_path = self.path.join("opt/pm/live");
        if live_path.exists() {
            return live_path; // Return the live path directly
        }

        // Legacy: Check for package-version directory
        let package_name = &self.manifest.package.name;
        let package_version = &self.manifest.package.version;
        let versioned_path = self.path.join(format!("{package_name}-{package_version}"));
        if versioned_path.exists() {
            return versioned_path;
        }

        // Fallback to package name without version
        self.path.join(package_name)
    }

    /// Get the blobs directory
    #[must_use]
    pub fn blobs_path(&self) -> PathBuf {
        self.path.join("blobs")
    }

    /// Link package contents to a destination
    ///
    /// For new file-level packages, this links from the file store.
    /// For legacy packages, this links from the package directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The files directory is missing or corrupted
    /// - File linking operations fail
    /// - Directory creation fails
    pub async fn link_to(&self, dest_root: &Path) -> Result<(), Error> {
        // Check if this is a new file-level package
        if let Some(file_hashes) = &self.file_hashes {
            // New file-level package - link from file store
            let store_base = self.path.parent().unwrap().parent().unwrap();
            let file_store = crate::FileStore::new(store_base);

            // Link all files from the file store
            file_store
                .link_files(file_hashes, &PathBuf::new(), dest_root)
                .await?;
            Ok(())
        } else {
            // Legacy package - link from package directory
            let files_dir = self.files_path();

            if !exists(&files_dir).await {
                return Err(PackageError::Corrupted {
                    message: "missing files directory".to_string(),
                }
                .into());
            }

            // Recursively link all files
            self.link_dir(&files_dir, dest_root).await
        }
    }

    async fn link_dir(&self, src: &Path, dest: &Path) -> Result<(), Error> {
        // Create destination directory
        create_dir_all(dest).await?;

        let mut entries = fs::read_dir(src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let src_path = entry.path();
            let file_name = entry.file_name();
            let dest_path = dest.join(&file_name);

            let metadata = entry.metadata().await?;

            if metadata.is_dir() {
                // Recursively link subdirectories
                Box::pin(self.link_dir(&src_path, &dest_path)).await?;
            } else if metadata.is_file() {
                // Hard link the file
                if exists(&dest_path).await {
                    // Remove existing file/link
                    fs::remove_file(&dest_path).await?;
                }
                hard_link(&src_path, &dest_path).await?;
            } else if metadata.is_symlink() {
                // Copy symlinks
                let target = fs::read_link(&src_path).await?;

                if exists(&dest_path).await {
                    fs::remove_file(&dest_path).await?;
                }

                #[cfg(unix)]
                {
                    use std::os::unix::fs::symlink;
                    symlink(&target, &dest_path)?;
                }
            }
        }

        Ok(())
    }

    /// Calculate total size of the package
    ///
    /// # Errors
    ///
    /// Returns an error if size calculation fails due to I/O issues
    pub async fn size(&self) -> Result<u64, Error> {
        size(&self.path).await
    }

    /// List all files in the package
    ///
    /// # Errors
    ///
    /// Returns an error if directory traversal fails or I/O operations fail
    pub async fn list_files(&self) -> Result<Vec<PathBuf>, Error> {
        let files_dir = self.files_path();
        if !exists(&files_dir).await {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        self.collect_files(&files_dir, &files_dir, &mut files)
            .await?;
        Ok(files)
    }

    async fn collect_files(
        &self,
        base: &Path,
        dir: &Path,
        files: &mut Vec<PathBuf>,
    ) -> Result<(), Error> {
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = entry.metadata().await?;

            if metadata.is_dir() {
                Box::pin(self.collect_files(base, &path, files)).await?;
            } else {
                // Store relative path
                if let Ok(rel_path) = path.strip_prefix(base) {
                    files.push(rel_path.to_path_buf());
                }
            }
        }

        Ok(())
    }

    /// Verify package integrity
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Required directories are missing
    /// - Manifest validation fails
    /// - Package structure is corrupted
    pub async fn verify(&self) -> Result<(), Error> {
        // Check required directories exist
        if !exists(&self.files_path()).await {
            return Err(PackageError::Corrupted {
                message: "missing files directory".to_string(),
            }
            .into());
        }

        // Validate manifest
        self.manifest.validate()?;

        // Could add more verification here (file checksums, etc.)

        Ok(())
    }
}

//! Stored package representation and operations

use spsv2_errors::{Error, PackageError};
use spsv2_manifest::Manifest;
use spsv2_root;
use std::path::{Path, PathBuf};
use tokio::fs;

/// A package stored in the content-addressed store
pub struct StoredPackage {
    path: PathBuf,
    manifest: Manifest,
}

impl StoredPackage {
    /// Load a stored package
    pub async fn load(path: &Path) -> Result<Self, Error> {
        let manifest_path = path.join("manifest.toml");
        let manifest = Manifest::from_file(&manifest_path).await?;

        Ok(Self {
            path: path.to_path_buf(),
            manifest,
        })
    }

    /// Get the package manifest
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Get the package path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the files directory
    pub fn files_path(&self) -> PathBuf {
        self.path.join("files")
    }

    /// Get the blobs directory
    pub fn blobs_path(&self) -> PathBuf {
        self.path.join("blobs")
    }

    /// Link package contents to a destination
    pub async fn link_to(&self, dest_root: &Path) -> Result<(), Error> {
        let files_dir = self.files_path();

        if !spsv2_root::exists(&files_dir).await {
            return Err(PackageError::Corrupted {
                message: "missing files directory".to_string(),
            }
            .into());
        }

        // Recursively link all files
        self.link_dir(&files_dir, dest_root).await
    }

    async fn link_dir(&self, src: &Path, dest: &Path) -> Result<(), Error> {
        // Create destination directory
        spsv2_root::create_dir_all(dest).await?;

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
                if spsv2_root::exists(&dest_path).await {
                    // Remove existing file/link
                    fs::remove_file(&dest_path).await?;
                }
                spsv2_root::hard_link(&src_path, &dest_path).await?;
            } else if metadata.is_symlink() {
                // Copy symlinks
                let target = fs::read_link(&src_path).await?;

                if spsv2_root::exists(&dest_path).await {
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
    pub async fn size(&self) -> Result<u64, Error> {
        spsv2_root::size(&self.path).await
    }

    /// List all files in the package
    pub async fn list_files(&self) -> Result<Vec<PathBuf>, Error> {
        let files_dir = self.files_path();
        if !spsv2_root::exists(&files_dir).await {
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
    pub async fn verify(&self) -> Result<(), Error> {
        // Check required directories exist
        if !spsv2_root::exists(&self.files_path()).await {
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

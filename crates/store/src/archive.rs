//! Package archive handling (.sp files)

use spsv2_errors::{Error, PackageError, StorageError};
use spsv2_root::{create_dir_all, exists};
use std::path::Path;
use tar::Archive;

/// Extract a .sp package file to a directory
///
/// # Errors
///
/// Returns an error if:
/// - Tar extraction fails
/// - The extracted package is missing manifest.toml
/// - I/O operations fail
pub async fn extract_package(sp_file: &Path, dest: &Path) -> Result<(), Error> {
    // For now, use simple tar extraction (can add zstd later)
    extract_tar_file(sp_file, dest).await?;

    // Verify manifest exists
    let manifest_path = dest.join("manifest.toml");
    if !exists(&manifest_path).await {
        return Err(PackageError::InvalidFormat {
            message: "missing manifest.toml in package".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Create a .sp package file from a directory
///
/// # Errors
///
/// Returns an error if:
/// - Source directory is missing manifest.toml
/// - Archive creation fails
/// - I/O operations fail
/// - Directory creation fails
pub async fn create_package(src: &Path, sp_file: &Path) -> Result<(), Error> {
    // Verify source has required structure
    let manifest_path = src.join("manifest.toml");
    if !exists(&manifest_path).await {
        return Err(PackageError::InvalidFormat {
            message: "source directory missing manifest.toml".to_string(),
        }
        .into());
    }

    // Create parent directory if needed
    if let Some(parent) = sp_file.parent() {
        create_dir_all(parent).await?;
    }

    // Create archive using blocking operations
    let src = src.to_path_buf();
    let sp_file = sp_file.to_path_buf();

    tokio::task::spawn_blocking(move || {
        use std::fs::File;
        use std::io::BufWriter;

        let file = File::create(&sp_file)?;
        let buf_writer = BufWriter::new(file);
        let mut builder = tar::Builder::new(buf_writer);

        // Set options for deterministic output
        builder.mode(tar::HeaderMode::Deterministic);
        builder.follow_symlinks(false);

        // Add all files from the source directory
        add_dir_to_tar(&mut builder, &src, Path::new(""))?;

        // Finish the archive
        builder.finish()?;

        Ok::<(), Error>(())
    })
    .await
    .map_err(|e| Error::internal(format!("create task failed: {e}")))??;

    Ok(())
}

/// Extract a tar archive from a file
async fn extract_tar_file(file_path: &Path, dest: &Path) -> Result<(), Error> {
    // Create destination directory
    create_dir_all(dest).await?;

    let file_path = file_path.to_path_buf();
    let dest = dest.to_path_buf();

    tokio::task::spawn_blocking(move || {
        use std::fs::File;

        let file = File::open(&file_path)?;
        let mut archive = Archive::new(file);

        // Set options for security
        archive.set_preserve_permissions(true);
        archive.set_preserve_mtime(true);
        archive.set_unpack_xattrs(false); // Don't unpack extended attributes

        // Extract all entries
        for entry in archive.entries()? {
            let mut entry = entry?;

            // Get the path
            let path = entry.path()?;

            // Security check: ensure path doesn't escape destination
            if path
                .components()
                .any(|c| c == std::path::Component::ParentDir)
            {
                return Err(PackageError::InvalidFormat {
                    message: "archive contains path traversal".to_string(),
                }
                .into());
            }

            // Unpack the entry
            entry.unpack_in(&dest)?;
        }

        Ok::<(), Error>(())
    })
    .await
    .map_err(|e| Error::internal(format!("extract task failed: {e}")))??;

    Ok(())
}

/// Recursively add directory contents to tar
fn add_dir_to_tar<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    src: &Path,
    prefix: &Path,
) -> Result<(), Error> {
    let entries = std::fs::read_dir(src).map_err(|e| StorageError::IoError {
        message: e.to_string(),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| StorageError::IoError {
            message: e.to_string(),
        })?;

        let path = entry.path();
        let name = entry.file_name();
        let tar_path = prefix.join(&name);

        let metadata = entry.metadata().map_err(|e| StorageError::IoError {
            message: e.to_string(),
        })?;

        if metadata.is_dir() {
            // Add directory
            builder
                .append_dir(&tar_path, &path)
                .map_err(|e| StorageError::IoError {
                    message: e.to_string(),
                })?;

            // Recursively add contents
            add_dir_to_tar(builder, &path, &tar_path)?;
        } else if metadata.is_file() {
            // Add file
            let mut file = std::fs::File::open(&path).map_err(|e| StorageError::IoError {
                message: e.to_string(),
            })?;

            builder
                .append_file(&tar_path, &mut file)
                .map_err(|e| StorageError::IoError {
                    message: e.to_string(),
                })?;
        } else if metadata.is_symlink() {
            // Add symlink
            let target = std::fs::read_link(&path).map_err(|e| StorageError::IoError {
                message: e.to_string(),
            })?;

            let mut header = tar::Header::new_gnu();
            header.set_metadata(&metadata);
            header.set_entry_type(tar::EntryType::Symlink);

            builder
                .append_link(&mut header, &tar_path, &target)
                .map_err(|e| StorageError::IoError {
                    message: e.to_string(),
                })?;
        }
    }

    Ok(())
}

//! Package archive handling (.sp files)
//!
//! This module provides support for .sp package archives using zstd compression.

use async_compression::tokio::bufread::ZstdDecoder as AsyncZstdReader;
use sps2_errors::{Error, PackageError, StorageError};
use sps2_events::{AppEvent, EventEmitter, EventSender, GeneralEvent};
use sps2_platform::core::PlatformContext;
use sps2_platform::Platform;
use std::path::Path;
use tar::Archive;
use tokio::io::{AsyncWriteExt, BufReader};

/// Create a platform context for filesystem operations
fn create_platform_context() -> (Platform, PlatformContext) {
    let platform = Platform::current();
    let context = platform.create_context(None);
    (platform, context)
}

/// Extract a .sp package file to a directory
///
/// # Errors
///
/// Returns an error if:
/// - Tar extraction fails
/// - The extracted package is missing manifest.toml
/// - I/O operations fail
pub async fn extract_package(sp_file: &Path, dest: &Path) -> Result<(), Error> {
    extract_package_with_events(sp_file, dest, None).await
}

/// Extract a .sp package file to a directory with optional event reporting
///
/// # Errors
///
/// Returns an error if:
/// - Tar extraction fails
/// - The extracted package is missing manifest.toml
/// - I/O operations fail
pub async fn extract_package_with_events(
    sp_file: &Path,
    dest: &Path,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    // Try zstd extraction first, fall back to plain tar if it fails
    match extract_zstd_tar_file(sp_file, dest, event_sender).await {
        Ok(()) => {}
        Err(_) => {
            // Fall back to plain tar
            extract_plain_tar_file(sp_file, dest, event_sender).await?;
        }
    }

    // Verify manifest exists
    let manifest_path = dest.join("manifest.toml");
    let (platform, ctx) = create_platform_context();
    if !platform.filesystem().exists(&ctx, &manifest_path).await {
        return Err(PackageError::InvalidFormat {
            message: "missing manifest.toml in package".to_string(),
        }
        .into());
    }

    Ok(())
}

/// List the contents of a .sp package without extracting
///
/// # Errors
///
/// Returns an error if:
/// - Archive reading fails
/// - I/O operations fail
pub async fn list_package_contents(sp_file: &Path) -> Result<Vec<String>, Error> {
    // Try zstd-compressed listing first, fall back to plain tar
    match list_zstd_tar_contents(sp_file).await {
        Ok(contents) => Ok(contents),
        Err(_) => list_plain_tar_contents(sp_file).await,
    }
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
    let (platform, ctx) = create_platform_context();

    if !platform.filesystem().exists(&ctx, &manifest_path).await {
        return Err(PackageError::InvalidFormat {
            message: "source directory missing manifest.toml".to_string(),
        }
        .into());
    }

    // Create parent directory if needed
    if let Some(parent) = sp_file.parent() {
        platform.filesystem().create_dir_all(&ctx, parent).await?;
    }

    // Create archive using blocking operations
    let src = src.to_path_buf();
    let sp_file = sp_file.to_path_buf();

    tokio::task::spawn_blocking(move || {
        use std::fs::OpenOptions;
        use std::io::Write;

        // Open file with create and write permissions
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&sp_file)
            .map_err(|e| StorageError::IoError {
                message: format!("failed to create file: {e}"),
            })?;

        let mut builder = tar::Builder::new(file);

        // Set options for deterministic output
        builder.mode(tar::HeaderMode::Deterministic);
        builder.follow_symlinks(false);

        // Add all files from the source directory
        add_dir_to_tar(&mut builder, &src, Path::new(""))?;

        // Finish the archive - this writes the tar EOF blocks
        builder.finish()?;

        // Get the file back and ensure it's synced
        let mut file = builder.into_inner().map_err(|e| StorageError::IoError {
            message: format!("failed to get file from tar builder: {e}"),
        })?;

        file.flush().map_err(|e| StorageError::IoError {
            message: format!("failed to flush file: {e}"),
        })?;

        file.sync_all().map_err(|e| StorageError::IoError {
            message: format!("failed to sync file: {e}"),
        })?;

        Ok::<(), Error>(())
    })
    .await
    .map_err(|e| Error::internal(format!("create task failed: {e}")))??;

    Ok(())
}

/// Extract a zstd-compressed tar archive using temporary file
async fn extract_zstd_tar_file(
    file_path: &Path,
    dest: &Path,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    // Create destination directory
    let (platform, ctx) = create_platform_context();
    platform.filesystem().create_dir_all(&ctx, dest).await?;

    // Create a temporary file to decompress to, then extract with tar
    let temp_file = tempfile::NamedTempFile::new().map_err(|e| StorageError::IoError {
        message: format!("failed to create temp file: {e}"),
    })?;

    let temp_path = temp_file.path().to_path_buf();

    // Decompress the zstd file to temporary location
    {
        use tokio::fs::File;

        let input_file = File::open(file_path)
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to open compressed file: {e}"),
            })?;

        let mut output_file =
            File::create(&temp_path)
                .await
                .map_err(|e| StorageError::IoError {
                    message: format!("failed to create temp output file: {e}"),
                })?;

        let mut decoder = AsyncZstdReader::new(BufReader::new(input_file));
        tokio::io::copy(&mut decoder, &mut output_file)
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to decompress zstd file: {e}"),
            })?;

        output_file
            .flush()
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to flush temp file: {e}"),
            })?;
    }

    // Now extract the decompressed tar file using blocking operations
    let temp_path_for_task = temp_path.clone();
    let dest = dest.to_path_buf();

    // Keep the temp_file alive until after the blocking operation completes
    tokio::task::spawn_blocking(move || {
        use std::fs::File;

        let file = File::open(&temp_path_for_task).map_err(|e| StorageError::IoError {
            message: format!("failed to open decompressed temp file: {e}"),
        })?;

        let mut archive = Archive::new(file);

        // Set options for security
        archive.set_preserve_permissions(true);
        archive.set_preserve_mtime(true);
        archive.set_unpack_xattrs(false); // Don't unpack extended attributes

        // Extract all entries with security checks
        extract_archive_entries(&mut archive, &dest)?;

        Ok::<(), Error>(())
    })
    .await
    .map_err(|e| Error::internal(format!("zstd extract task failed: {e}")))??;

    // Now we can safely drop the temp_file
    drop(temp_file);

    // Send decompression completed event
    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::OperationCompleted {
            operation: "Zstd decompression completed".to_string(),
            success: true,
        }));
    }

    // Send overall extraction completed event
    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::OperationCompleted {
            operation: format!("Package extraction completed: {}", file_path.display()),
            success: true,
        }));
    }

    Ok(())
}

/// Extract a plain (uncompressed) tar archive
async fn extract_plain_tar_file(
    file_path: &Path,
    dest: &Path,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    // Create destination directory
    let (platform, ctx) = create_platform_context();
    platform.filesystem().create_dir_all(&ctx, dest).await?;

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

        // Extract all entries with security checks
        extract_archive_entries(&mut archive, &dest)?;

        Ok::<(), Error>(())
    })
    .await
    .map_err(|e| Error::internal(format!("plain tar extract task failed: {e}")))??;

    // Send extraction completed event
    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::OperationCompleted {
            operation: "Plain tar extraction completed".to_string(),
            success: true,
        }));
    }

    Ok(())
}

/// List contents of a zstd-compressed tar archive
async fn list_zstd_tar_contents(file_path: &Path) -> Result<Vec<String>, Error> {
    // Create a temporary file to decompress to, then list contents
    let temp_file = tempfile::NamedTempFile::new().map_err(|e| StorageError::IoError {
        message: format!("failed to create temp file: {e}"),
    })?;

    let temp_path = temp_file.path().to_path_buf();

    // Decompress the zstd file to temporary location
    {
        use tokio::fs::File;

        let input_file = File::open(file_path)
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to open compressed file: {e}"),
            })?;

        let mut output_file =
            File::create(&temp_path)
                .await
                .map_err(|e| StorageError::IoError {
                    message: format!("failed to create temp output file: {e}"),
                })?;

        let mut decoder = AsyncZstdReader::new(BufReader::new(input_file));
        tokio::io::copy(&mut decoder, &mut output_file)
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to decompress zstd file: {e}"),
            })?;

        output_file
            .flush()
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to flush temp file: {e}"),
            })?;
    }

    // Now list the decompressed tar file contents
    let temp_path_for_task = temp_path.clone();

    // Keep temp_file alive until after the blocking operation completes
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<String>, Error> {
        use std::fs::File;

        let file = File::open(&temp_path_for_task)?;
        let mut archive = Archive::new(file);
        let mut files = Vec::new();

        for entry in archive.entries()? {
            let entry = entry?;
            let path = entry.path()?;
            files.push(path.to_string_lossy().to_string());
        }

        files.sort();
        Ok(files)
    })
    .await
    .map_err(|e| Error::internal(format!("zstd list task failed: {e}")))?;

    // Now we can safely drop the temp_file
    drop(temp_file);

    result
}

/// List contents of a plain tar file
async fn list_plain_tar_contents(file_path: &Path) -> Result<Vec<String>, Error> {
    let file_path = file_path.to_path_buf();

    tokio::task::spawn_blocking(move || -> Result<Vec<String>, Error> {
        use std::fs::File;

        let file = File::open(&file_path)?;
        let mut archive = Archive::new(file);
        let mut files = Vec::new();

        for entry in archive.entries()? {
            let entry = entry?;
            let path = entry.path()?;
            files.push(path.to_string_lossy().to_string());
        }

        files.sort();
        Ok(files)
    })
    .await
    .map_err(|e| Error::internal(format!("plain tar list task failed: {e}")))?
}

/// Extract entries from a tar archive with security checks
fn extract_archive_entries<R: std::io::Read>(
    archive: &mut Archive<R>,
    dest: &Path,
) -> Result<(), Error> {
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
        entry.unpack_in(dest)?;
    }

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

#![allow(clippy::module_name_repetitions, unused_variables)]

//! Filesystem convenience helpers backed by the platform abstraction.
//!
//! These functions provide a stable API returning `sps2_errors::Error` while
//! internally leveraging the platform's `FilesystemOperations` implementation.

use crate::PlatformManager;
use sps2_errors::{Error, StorageError};
use std::path::Path;
use tokio::fs;
use tokio::task;

/// Result type for filesystem operations
pub type Result<T> = std::result::Result<T, sps2_errors::Error>;

/// APFS clonefile support
#[cfg(target_os = "macos")]
mod apfs {
    use super::{Path, Result, StorageError};
    use crate::PlatformManager;

    /// Clone a file or directory using APFS clonefile with security flags
    pub async fn clone_path(src: &Path, dst: &Path) -> Result<()> {
        let platform = PlatformManager::instance().platform();
        let context = platform.create_context(None);

        // Use clone_directory for directories, clone_file for files
        let metadata = std::fs::metadata(src).map_err(|e| StorageError::IoError {
            message: e.to_string(),
        })?;

        let result = if metadata.is_dir() {
            platform
                .filesystem()
                .clone_directory(&context, src, dst)
                .await
        } else {
            platform.filesystem().clone_file(&context, src, dst).await
        };

        result.map_err(|platform_err| {
            // Convert PlatformError back to StorageError for backward compatibility
            match platform_err {
                sps2_errors::PlatformError::FilesystemOperationFailed { message, .. } => {
                    StorageError::ApfsCloneFailed { message }.into()
                }
                _ => StorageError::ApfsCloneFailed {
                    message: platform_err.to_string(),
                }
                .into(),
            }
        })
    }
}

/// Atomic rename with swap support
///
/// This function uses macOS `renamex_np` with `RENAME_SWAP` when both paths exist,
/// or regular rename when only source exists. This provides true atomic swap
/// behavior critical for system stability during updates.
///
/// # Errors
///
/// Returns an error if:
/// - Path conversion to C string fails
/// - The atomic rename operation fails (permissions, file not found, etc.)
/// - The blocking task panics
pub async fn atomic_rename(src: &Path, dst: &Path) -> Result<()> {
    let platform = PlatformManager::instance().platform();
    let context = platform.create_context(None);

    platform
        .filesystem()
        .atomic_rename(&context, src, dst)
        .await
        .map_err(|platform_err| {
            // Convert PlatformError back to StorageError for backward compatibility
            match platform_err {
                sps2_errors::PlatformError::FilesystemOperationFailed { message, .. } => {
                    StorageError::AtomicRenameFailed { message }.into()
                }
                _ => StorageError::AtomicRenameFailed {
                    message: platform_err.to_string(),
                }
                .into(),
            }
        })
}

/// True atomic swap that requires both paths to exist
///
/// This function guarantees atomic exchange of two directories/files using
/// platform abstraction. This is critical for rollback operations
/// where we need to swap live and backup directories atomically.
///
/// # Errors
///
/// Returns an error if:
/// - Either path doesn't exist
/// - The atomic swap operation fails
pub async fn atomic_swap(path1: &Path, path2: &Path) -> Result<()> {
    let platform = PlatformManager::instance().platform();
    let context = platform.create_context(None);

    platform
        .filesystem()
        .atomic_swap(&context, path1, path2)
        .await
        .map_err(|platform_err| {
            // Convert PlatformError back to StorageError for backward compatibility
            match platform_err {
                sps2_errors::PlatformError::FilesystemOperationFailed { message, .. } => {
                    StorageError::AtomicRenameFailed { message }.into()
                }
                _ => StorageError::AtomicRenameFailed {
                    message: platform_err.to_string(),
                }
                .into(),
            }
        })
}

/// Clone a directory tree using APFS clonefile
///
/// # Errors
///
/// Returns an error if:
/// - Path conversion to C string fails
/// - The APFS clonefile operation fails (permissions, insufficient space, etc.)
/// - The blocking task panics
#[cfg(target_os = "macos")]
pub async fn clone_directory(src: &Path, dst: &Path) -> Result<()> {
    apfs::clone_path(src, dst).await
}

/// Clone a directory tree (fallback for non-APFS)
///
/// # Errors
///
/// Returns an error if the recursive copy operation fails
#[cfg(not(target_os = "macos"))]
pub async fn clone_directory(src: &Path, dst: &Path) -> Result<()> {
    // Fallback to recursive copy
    copy_directory(src, dst).await
}

/// Recursively copy a directory
///
/// # Errors
///
/// Returns an error if:
/// - Creating the destination directory fails
/// - Reading the source directory fails
/// - Copying any file or subdirectory fails
pub async fn copy_directory(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).await?;

    let mut entries = fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        let metadata = entry.metadata().await?;
        if metadata.is_dir() {
            Box::pin(copy_directory(&src_path, &dst_path)).await?;
        } else {
            fs::copy(&src_path, &dst_path).await?;
        }
    }

    Ok(())
}

/// Create a directory with all parent directories
///
/// # Errors
///
/// Returns an error if:
/// - Permission is denied
/// - Any I/O operation fails during directory creation
pub async fn create_dir_all(path: &Path) -> Result<()> {
    let platform = PlatformManager::instance().platform();
    let context = platform.create_context(None);

    platform
        .filesystem()
        .create_dir_all(&context, path)
        .await
        .map_err(|platform_err| {
            // Convert PlatformError back to StorageError for backward compatibility
            match platform_err {
                sps2_errors::PlatformError::FilesystemOperationFailed { message, .. } => {
                    StorageError::IoError { message }.into()
                }
                _ => StorageError::IoError {
                    message: platform_err.to_string(),
                }
                .into(),
            }
        })
}

/// Remove a directory and all its contents
///
/// # Errors
///
/// Returns an error if:
/// - The directory removal operation fails (permissions, non-empty directory, etc.)
pub async fn remove_dir_all(path: &Path) -> Result<()> {
    let platform = PlatformManager::instance().platform();
    let context = platform.create_context(None);

    platform
        .filesystem()
        .remove_dir_all(&context, path)
        .await
        .map_err(|platform_err| {
            // Convert PlatformError back to StorageError for backward compatibility
            match platform_err {
                sps2_errors::PlatformError::FilesystemOperationFailed { message, .. } => {
                    StorageError::IoError { message }.into()
                }
                _ => StorageError::IoError {
                    message: platform_err.to_string(),
                }
                .into(),
            }
        })
}

/// Create a hard link with platform optimization
///
/// # Errors
///
/// Returns an error if:
/// - The source file does not exist
/// - The destination already exists
/// - The hard link operation fails (cross-device link, permissions, etc.)
pub async fn hard_link(src: &Path, dst: &Path) -> Result<()> {
    let platform = PlatformManager::instance().platform();
    let context = platform.create_context(None);

    platform
        .filesystem()
        .hard_link(&context, src, dst)
        .await
        .map_err(|platform_err| {
            // Convert PlatformError back to StorageError for backward compatibility
            match platform_err {
                sps2_errors::PlatformError::FilesystemOperationFailed { message, .. } => {
                    StorageError::IoError { message }.into()
                }
                _ => StorageError::IoError {
                    message: platform_err.to_string(),
                }
                .into(),
            }
        })
}

/// Create staging directory using platform-optimized methods
///
/// This function will clone an existing live directory if it exists,
/// or create a new empty directory for fresh installations.
///
/// # Errors
///
/// Returns an error if:
/// - Directory creation fails
/// - Clone operation fails
/// - Parent directory creation fails
pub async fn create_staging_directory(live_path: &Path, staging_path: &Path) -> Result<()> {
    if exists(live_path).await {
        // Ensure parent directory exists for staging path
        if let Some(parent) = staging_path.parent() {
            create_dir_all(parent).await?;
        }

        // Remove staging directory if it already exists (clonefile requires destination to not exist)
        if exists(staging_path).await {
            remove_dir_all(staging_path).await?;
        }

        // Clone the live directory to staging
        clone_directory(live_path, staging_path).await?;
    } else {
        // Create empty staging directory for fresh installation
        create_dir_all(staging_path).await?;
    }

    Ok(())
}

/// Rename a file or directory
///
/// # Errors
///
/// Returns an error if:
/// - The rename operation fails (permissions, cross-device, etc.)
pub async fn rename(src: &Path, dst: &Path) -> Result<()> {
    fs::rename(src, dst).await.map_err(|e| {
        StorageError::IoError {
            message: format!("rename failed: {e}"),
        }
        .into()
    })
}

/// Check if a path exists
pub async fn exists(path: &Path) -> bool {
    let platform = PlatformManager::instance().platform();
    let context = platform.create_context(None);

    platform.filesystem().exists(&context, path).await
}

/// Get the size of a file or directory
///
/// # Errors
///
/// Returns an error if:
/// - Reading file metadata fails
/// - Reading directory contents fails
/// - Any I/O operation fails during recursive directory traversal
pub async fn size(path: &Path) -> Result<u64> {
    let platform = PlatformManager::instance().platform();
    let context = platform.create_context(None);

    platform
        .filesystem()
        .size(&context, path)
        .await
        .map_err(|platform_err| {
            // Convert PlatformError back to StorageError for backward compatibility
            match platform_err {
                sps2_errors::PlatformError::FilesystemOperationFailed { message, .. } => {
                    StorageError::IoError { message }.into()
                }
                _ => StorageError::IoError {
                    message: platform_err.to_string(),
                }
                .into(),
            }
        })
}

/// Ensure a directory exists and is empty
///
/// # Errors
///
/// Returns an error if:
/// - Directory removal fails
/// - Directory creation fails
pub async fn ensure_empty_dir(path: &Path) -> Result<()> {
    if exists(path).await {
        remove_dir_all(path).await?;
    }
    create_dir_all(path).await
}

/// Set APFS compression attribute on a path (macOS only)
#[cfg(target_os = "macos")]
pub fn set_compression(_path: &Path) -> Result<()> {
    // Placeholder for compression extended attributes
    Ok(())
}

/// Set APFS compression attribute on a path (no-op on non-macOS)
#[cfg(not(target_os = "macos"))]
pub fn set_compression(_path: &Path) -> Result<()> {
    Ok(())
}

/// Remove a single file
///
/// # Errors
///
/// Returns an error if:
/// - The file removal operation fails (permissions, file not found, etc.)
pub async fn remove_file(path: &Path) -> Result<()> {
    let platform = PlatformManager::instance().platform();
    let context = platform.create_context(None);

    platform
        .filesystem()
        .remove_file(&context, path)
        .await
        .map_err(|platform_err| {
            // Convert PlatformError back to StorageError for backward compatibility
            match platform_err {
                sps2_errors::PlatformError::FilesystemOperationFailed { message, .. } => {
                    StorageError::IoError { message }.into()
                }
                _ => StorageError::IoError {
                    message: platform_err.to_string(),
                }
                .into(),
            }
        })
}

#[cfg(unix)]
fn symlink_inner(target: &Path, link: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::symlink;

    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if let Ok(metadata) = std::fs::symlink_metadata(link) {
        if metadata.file_type().is_dir() {
            std::fs::remove_dir_all(link)?;
        } else {
            std::fs::remove_file(link)?;
        }
    }

    symlink(target, link)
}

/// Create a symbolic link using platform-safe operations (Unix only).
///
/// # Errors
///
/// Returns an error if:
/// - Creating parent directories fails
/// - Removing a pre-existing path at the link location fails
/// - The symlink creation itself fails
#[cfg(unix)]
pub async fn symlink(target: &Path, link: &Path) -> Result<()> {
    let target = target.to_path_buf();
    let link = link.to_path_buf();

    let result = task::spawn_blocking(move || symlink_inner(&target, &link))
        .await
        .map_err(|e| {
            Error::from(StorageError::IoError {
                message: format!("symlink task failed: {e}"),
            })
        })?;

    result
        .map_err(|e| {
            Error::from(StorageError::IoError {
                message: format!("symlink operation failed: {e}"),
            })
        })?;

    Ok(())
}

/// Create a symbolic link (not supported on non-Unix platforms).
#[cfg(not(unix))]
pub async fn symlink(_target: &Path, _link: &Path) -> Result<()> {
    Err(StorageError::IoError {
        message: "symlink is not supported on this platform".to_string(),
    }
    .into())
}

#[cfg(unix)]
async fn remove_tmp_link(path: &Path) -> Result<()> {
    if !exists(path).await {
        return Ok(());
    }

    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_dir() => remove_dir_all(path).await?,
        Ok(_) => remove_file(path).await?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(StorageError::IoError {
                message: format!("failed to inspect temporary link: {e}"),
            }
            .into())
        }
    }

    Ok(())
}

/// Atomically replace a symbolic link with a new target (Unix only).
///
/// This helper creates a temporary symlink pointing at `target` and swaps it into
/// place using the platform's atomic rename primitive.
#[cfg(unix)]
pub async fn replace_symlink(target: &Path, link: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        create_dir_all(parent).await?;
    }

    let temp_link = link.with_extension("tmp-link");
    remove_tmp_link(&temp_link).await?;

    symlink(target, &temp_link).await?;
    atomic_rename(&temp_link, link).await
}

/// Atomically replace a symbolic link with a new target (not supported on non-Unix platforms).
#[cfg(not(unix))]
pub async fn replace_symlink(_target: &Path, _link: &Path) -> Result<()> {
    Err(StorageError::IoError {
        message: "replace_symlink is not supported on this platform".to_string(),
    }
    .into())
}

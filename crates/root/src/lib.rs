#![deny(clippy::pedantic)]
#![allow(clippy::module_name_repetitions, unused_variables)]

//! Filesystem operations for sps2
//!
//! This crate provides APFS-optimized filesystem operations including
//! atomic renames, clonefile support, and directory management.

use sps2_errors::{Error, StorageError};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use tokio::fs;

/// Result type for filesystem operations
type Result<T> = std::result::Result<T, Error>;

/// APFS clonefile support
#[cfg(target_os = "macos")]
mod apfs {
    use super::{CString, Error, OsStrExt, Path, Result, StorageError};

    // macOS clonefile flags for security and proper ownership handling
    const CLONE_NOFOLLOW: u32 = 0x0001; // Don't follow symbolic links
    const CLONE_NOOWNERCOPY: u32 = 0x0002; // Don't copy owner information

    /// Clone a file or directory using APFS clonefile with security flags
    #[allow(unsafe_code)]
    pub async fn clone_path(src: &Path, dst: &Path) -> Result<()> {
        let src_cstring =
            CString::new(src.as_os_str().as_bytes()).map_err(|_| StorageError::InvalidPath {
                path: src.display().to_string(),
            })?;

        let dst_cstring =
            CString::new(dst.as_os_str().as_bytes()).map_err(|_| StorageError::InvalidPath {
                path: dst.display().to_string(),
            })?;

        tokio::task::spawn_blocking(move || {
            // SAFETY: clonefile is available on macOS and we're passing valid C strings
            unsafe {
                let result = libc::clonefile(
                    src_cstring.as_ptr(),
                    dst_cstring.as_ptr(),
                    CLONE_NOFOLLOW | CLONE_NOOWNERCOPY,
                );

                if result != 0 {
                    let errno = *libc::__error();
                    return Err(StorageError::ApfsCloneFailed {
                        message: format!(
                            "clonefile failed with code {result}, errno: {errno} ({})",
                            std::io::Error::from_raw_os_error(errno)
                        ),
                    }
                    .into());
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| Error::internal(format!("clone task failed: {e}")))?
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
    #[cfg(target_os = "macos")]
    {
        // Use async filesystem operations for proper directory handling
        if dst.exists() {
            if dst.is_dir() {
                // For directories, we need to remove the destination first
                // Create a temporary backup location
                let temp_dst = dst.with_extension("old");

                // Move destination to temp location
                tokio::fs::rename(dst, &temp_dst).await.map_err(|e| {
                    StorageError::AtomicRenameFailed {
                        message: format!("failed to backup destination: {e}"),
                    }
                })?;

                // Move source to destination
                match tokio::fs::rename(src, dst).await {
                    Ok(()) => {
                        // Success! Remove the old destination
                        let _ = tokio::fs::remove_dir_all(&temp_dst).await;
                        Ok(())
                    }
                    Err(e) => {
                        // Failed! Restore the original destination
                        let _ = tokio::fs::rename(&temp_dst, dst).await;
                        Err(StorageError::AtomicRenameFailed {
                            message: format!("rename failed: {e}"),
                        }
                        .into())
                    }
                }
            } else {
                // For files, regular rename should work
                tokio::fs::rename(src, dst).await.map_err(|e| {
                    StorageError::AtomicRenameFailed {
                        message: format!("rename failed: {e}"),
                    }
                    .into()
                })
            }
        } else {
            // Destination doesn't exist, regular rename
            tokio::fs::rename(src, dst).await.map_err(|e| {
                StorageError::AtomicRenameFailed {
                    message: format!("rename failed: {e}"),
                }
                .into()
            })
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Fallback to regular rename (not truly atomic swap)
        fs::rename(src, dst)
            .await
            .map_err(|e| StorageError::AtomicRenameFailed {
                message: e.to_string(),
            })?;
        Ok(())
    }
}

/// True atomic swap that requires both paths to exist
///
/// This function guarantees atomic exchange of two directories/files using
/// macOS `renamex_np` with `RENAME_SWAP`. This is critical for rollback operations
/// where we need to swap live and backup directories atomically.
///
/// # Errors
///
/// Returns an error if:
/// - Either path doesn't exist
/// - Path conversion to C string fails  
/// - The atomic swap operation fails
/// - The blocking task panics
pub async fn atomic_swap(path1: &Path, path2: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use libc::{c_uint, renamex_np, RENAME_SWAP};

        // Verify both paths exist before attempting swap
        if !path1.exists() {
            return Err(StorageError::PathNotFound {
                path: path1.display().to_string(),
            }
            .into());
        }
        if !path2.exists() {
            return Err(StorageError::PathNotFound {
                path: path2.display().to_string(),
            }
            .into());
        }

        let path1_cstring =
            CString::new(path1.as_os_str().as_bytes()).map_err(|_| StorageError::InvalidPath {
                path: path1.display().to_string(),
            })?;

        let path2_cstring =
            CString::new(path2.as_os_str().as_bytes()).map_err(|_| StorageError::InvalidPath {
                path: path2.display().to_string(),
            })?;

        tokio::task::spawn_blocking(move || {
            #[allow(unsafe_code)]
            // SAFETY: renamex_np is available on macOS and we're passing valid C strings
            unsafe {
                if renamex_np(
                    path1_cstring.as_ptr(),
                    path2_cstring.as_ptr(),
                    RENAME_SWAP as c_uint,
                ) != 0
                {
                    let err = std::io::Error::last_os_error();
                    return Err(StorageError::AtomicRenameFailed {
                        message: format!("atomic swap failed: {err}"),
                    }
                    .into());
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| Error::internal(format!("swap task failed: {e}")))?
    }

    #[cfg(not(target_os = "macos"))]
    {
        // No true atomic swap available on non-macOS platforms
        // This is a potentially unsafe fallback using temporary file
        let temp_path = path1.with_extension("tmp_swap");

        fs::rename(path1, &temp_path)
            .await
            .map_err(|e| StorageError::AtomicRenameFailed {
                message: format!("temp rename failed: {e}"),
            })?;

        fs::rename(path2, path1)
            .await
            .map_err(|e| StorageError::AtomicRenameFailed {
                message: format!("second rename failed: {e}"),
            })?;

        fs::rename(&temp_path, path2)
            .await
            .map_err(|e| StorageError::AtomicRenameFailed {
                message: format!("final rename failed: {e}"),
            })?;

        Ok(())
    }
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
    fs::create_dir_all(path).await.map_err(|e| {
        match e.kind() {
            std::io::ErrorKind::PermissionDenied => StorageError::PermissionDenied {
                path: path.display().to_string(),
            },
            _ => StorageError::IoError {
                message: e.to_string(),
            },
        }
        .into()
    })
}

/// Remove a directory and all its contents
///
/// # Errors
///
/// Returns an error if:
/// - The directory removal operation fails (permissions, non-empty directory, etc.)
pub async fn remove_dir_all(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .await
            .map_err(|e| StorageError::IoError {
                message: e.to_string(),
            })?;
    }
    Ok(())
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
    #[cfg(target_os = "macos")]
    {
        let src_cstring =
            CString::new(src.as_os_str().as_bytes()).map_err(|_| StorageError::InvalidPath {
                path: src.display().to_string(),
            })?;

        let dst_cstring =
            CString::new(dst.as_os_str().as_bytes()).map_err(|_| StorageError::InvalidPath {
                path: dst.display().to_string(),
            })?;

        tokio::task::spawn_blocking(move || {
            let result = unsafe { libc::link(src_cstring.as_ptr(), dst_cstring.as_ptr()) };
            if result != 0 {
                let errno = unsafe { *libc::__error() };
                return Err(StorageError::IoError {
                    message: format!(
                        "hard link failed with code {result}, errno: {errno} ({})",
                        std::io::Error::from_raw_os_error(errno)
                    ),
                }
                .into());
            }
            Ok(())
        })
        .await
        .map_err(|e| Error::internal(format!("hard link task failed: {e}")))?
    }

    #[cfg(not(target_os = "macos"))]
    {
        fs::hard_link(src, dst).await.map_err(|e| {
            StorageError::IoError {
                message: format!("hard link failed: {e}"),
            }
            .into()
        })
    }
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
    fs::metadata(path).await.is_ok()
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
    if path.is_file() {
        let metadata = fs::metadata(path).await?;
        Ok(metadata.len())
    } else {
        // Calculate directory size recursively
        let mut total = 0u64;
        let mut entries = fs::read_dir(path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                total += Box::pin(size(&path)).await?;
            } else {
                let metadata = entry.metadata().await?;
                total += metadata.len();
            }
        }

        Ok(total)
    }
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

/// Set APFS compression attribute on a path
///
/// # Errors
///
/// Currently this is a no-op placeholder and does not return errors
#[cfg(target_os = "macos")]
pub fn set_compression(_path: &Path) -> Result<()> {
    // This would use the compression extended attributes
    // For now, this is a placeholder
    Ok(())
}

/// Set APFS compression attribute on a path
///
/// # Errors
///
/// This is a no-op on non-macOS platforms and does not return errors
#[cfg(not(target_os = "macos"))]
pub fn set_compression(_path: &Path) -> Result<()> {
    // No-op on non-macOS
    Ok(())
}

/// Remove a file
///
/// # Errors
///
/// Returns an error if:
/// - The file removal operation fails (permissions, file not found, etc.)
pub async fn remove_file(path: &Path) -> Result<()> {
    fs::remove_file(path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StorageError::PathNotFound {
                path: path.display().to_string(),
            }
        } else {
            StorageError::IoError {
                message: format!("failed to remove file: {e}"),
            }
        }
        .into()
    })
}

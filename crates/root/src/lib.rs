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
    use libc::{c_char, c_int};

    extern "C" {
        fn clonefile(src: *const c_char, dst: *const c_char, flags: c_int) -> c_int;
    }

    /// Clone a file or directory using APFS clonefile
    #[allow(unsafe_code)]
    pub async fn clone_path(src: &Path, dst: &Path) -> Result<()> {
        let src_cstring =
            CString::new(src.as_os_str().as_bytes()).map_err(|e| StorageError::InvalidPath {
                path: src.display().to_string(),
            })?;

        let dst_cstring =
            CString::new(dst.as_os_str().as_bytes()).map_err(|e| StorageError::InvalidPath {
                path: dst.display().to_string(),
            })?;

        tokio::task::spawn_blocking(move || {
            // SAFETY: clonefile is available on macOS and we're passing valid C strings
            unsafe {
                if clonefile(src_cstring.as_ptr(), dst_cstring.as_ptr(), 0) != 0 {
                    let err = std::io::Error::last_os_error();
                    return Err(StorageError::ApfsCloneFailed {
                        message: err.to_string(),
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
/// # Errors
///
/// Returns an error if:
/// - Path conversion to C string fails
/// - The atomic rename operation fails (permissions, file not found, etc.)
/// - The blocking task panics
pub async fn atomic_rename(src: &Path, dst: &Path) -> Result<()> {
    // On macOS, we can use renamex_np with RENAME_SWAP flag
    #[cfg(target_os = "macos")]
    {
        use libc::{c_uint, renamex_np, RENAME_SWAP};

        let src_cstring =
            CString::new(src.as_os_str().as_bytes()).map_err(|e| StorageError::InvalidPath {
                path: src.display().to_string(),
            })?;

        let dst_cstring =
            CString::new(dst.as_os_str().as_bytes()).map_err(|e| StorageError::InvalidPath {
                path: dst.display().to_string(),
            })?;

        tokio::task::spawn_blocking(move || {
            #[allow(unsafe_code)]
            // SAFETY: renamex_np is available on macOS and we're passing valid C strings
            unsafe {
                if renamex_np(
                    src_cstring.as_ptr(),
                    dst_cstring.as_ptr(),
                    RENAME_SWAP as c_uint,
                ) != 0
                {
                    let err = std::io::Error::last_os_error();
                    return Err(StorageError::AtomicRenameFailed {
                        message: err.to_string(),
                    }
                    .into());
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| Error::internal(format!("rename task failed: {e}")))?
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

/// Create a hard link
///
/// # Errors
///
/// Returns an error if:
/// - The source file does not exist
/// - The destination already exists
/// - The hard link operation fails (cross-device link, permissions, etc.)
pub async fn hard_link(src: &Path, dst: &Path) -> Result<()> {
    fs::hard_link(src, dst).await.map_err(|e| {
        StorageError::IoError {
            message: format!("hard link failed: {e}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_and_remove_dir() {
        let temp = tempdir().unwrap();
        let test_dir = temp.path().join("test");

        create_dir_all(&test_dir).await.unwrap();
        assert!(exists(&test_dir).await);

        remove_dir_all(&test_dir).await.unwrap();
        assert!(!exists(&test_dir).await);
    }

    #[tokio::test]
    async fn test_hard_link() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst.txt");

        fs::write(&src, b"test content").await.unwrap();
        hard_link(&src, &dst).await.unwrap();

        let content = fs::read(&dst).await.unwrap();
        assert_eq!(content, b"test content");
    }

    #[tokio::test]
    async fn test_directory_size() {
        let temp = tempdir().unwrap();
        let dir = temp.path();

        fs::write(dir.join("file1.txt"), b"hello").await.unwrap();
        fs::write(dir.join("file2.txt"), b"world").await.unwrap();

        let total_size = size(dir).await.unwrap();
        assert_eq!(total_size, 10); // "hello" + "world"
    }
}

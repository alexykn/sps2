//! Platform-specific filesystem operations for atomic installations

use sps2_errors::{Error, InstallError};
use std::path::Path;
use tokio::fs;

/// APFS clonefile implementation for macOS
#[cfg(target_os = "macos")]
pub fn apfs_clonefile(source: &Path, dest: &Path) -> Result<(), Error> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // macOS clonefile flags
    const CLONE_NOFOLLOW: u32 = 0x0001;
    const CLONE_NOOWNERCOPY: u32 = 0x0002;

    let source_c =
        CString::new(source.as_os_str().as_bytes()).map_err(|_| InstallError::FilesystemError {
            operation: "clonefile".to_string(),
            path: source.display().to_string(),
            message: "invalid source path".to_string(),
        })?;

    let dest_c =
        CString::new(dest.as_os_str().as_bytes()).map_err(|_| InstallError::FilesystemError {
            operation: "clonefile".to_string(),
            path: dest.display().to_string(),
            message: "invalid dest path".to_string(),
        })?;

    // Use the proper clonefile function from libc, not syscall
    let result = unsafe {
        libc::clonefile(
            source_c.as_ptr(),
            dest_c.as_ptr(),
            CLONE_NOFOLLOW | CLONE_NOOWNERCOPY,
        )
    };

    if result != 0 {
        let errno = unsafe { *libc::__error() };
        return Err(InstallError::FilesystemError {
            operation: "clonefile".to_string(),
            path: format!("{} -> {}", source.display(), dest.display()),
            message: format!(
                "clonefile failed with code {result}, errno: {errno} ({})",
                std::io::Error::from_raw_os_error(errno)
            ),
        }
        .into());
    }

    Ok(())
}

/// Fallback directory copy for non-APFS filesystems
#[allow(dead_code)]
pub async fn copy_directory_recursive(source: &Path, dest: &Path) -> Result<(), Error> {
    fs::create_dir_all(dest).await?;

    let mut entries = fs::read_dir(source).await?;
    while let Some(entry) = entries.next_entry().await? {
        let entry_path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dest.join(&file_name);

        if entry_path.is_dir() {
            Box::pin(copy_directory_recursive(&entry_path, &dest_path)).await?;
        } else {
            fs::copy(&entry_path, &dest_path).await?;
        }
    }

    Ok(())
}

/// Create staging directory using platform-optimized methods
pub fn create_staging_directory(live_path: &Path, staging_path: &Path) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        if live_path.exists() {
            // Ensure parent directory exists for staging path
            if let Some(parent) = staging_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| InstallError::FilesystemError {
                    operation: "create_staging_parent".to_string(),
                    path: parent.display().to_string(),
                    message: e.to_string(),
                })?;
            }

            // Remove staging directory if it already exists (clonefile requires destination to not exist)
            if staging_path.exists() {
                std::fs::remove_dir_all(staging_path).map_err(|e| {
                    InstallError::FilesystemError {
                        operation: "remove_existing_staging".to_string(),
                        path: staging_path.display().to_string(),
                        message: e.to_string(),
                    }
                })?;
            }

            // Use APFS clonefile for instant, space-efficient copy
            apfs_clonefile(live_path, staging_path)?;
        } else {
            // Create empty staging directory for fresh installation
            std::fs::create_dir_all(staging_path).map_err(|e| InstallError::FilesystemError {
                operation: "create_staging_dir".to_string(),
                path: staging_path.display().to_string(),
                message: e.to_string(),
            })?;
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Fallback for non-macOS platforms
        if live_path.exists() {
            // Use standard directory copy for non-APFS filesystems
            std::fs::create_dir_all(staging_path).map_err(|e| InstallError::FilesystemError {
                operation: "create_staging_dir".to_string(),
                path: staging_path.display().to_string(),
                message: e.to_string(),
            })?;

            // Note: This is a placeholder for proper recursive copy implementation
            // In production, we would need async recursive copy or use tokio::task::spawn_blocking
        } else {
            std::fs::create_dir_all(staging_path).map_err(|e| InstallError::FilesystemError {
                operation: "create_staging_dir".to_string(),
                path: staging_path.display().to_string(),
                message: e.to_string(),
            })?;
        }
    }

    Ok(())
}

//! Hard link creation utilities for atomic installations

use sps2_errors::{Error, InstallError};
use std::path::Path;
use tokio::fs;

/// Create hard link (APFS-optimized on macOS)
#[cfg(target_os = "macos")]
pub fn create_hard_link(source: &Path, dest: &Path) -> Result<(), Error> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source_c =
        CString::new(source.as_os_str().as_bytes()).map_err(|_| InstallError::FilesystemError {
            operation: "create_hard_link".to_string(),
            path: source.display().to_string(),
            message: "invalid path".to_string(),
        })?;

    let dest_c =
        CString::new(dest.as_os_str().as_bytes()).map_err(|_| InstallError::FilesystemError {
            operation: "create_hard_link".to_string(),
            path: dest.display().to_string(),
            message: "invalid path".to_string(),
        })?;

    let result = unsafe { libc::link(source_c.as_ptr(), dest_c.as_ptr()) };

    if result != 0 {
        return Err(InstallError::FilesystemError {
            operation: "create_hard_link".to_string(),
            path: source.display().to_string(),
            message: format!("link failed with code {result}"),
        }
        .into());
    }

    Ok(())
}

/// Create hard link (fallback for non-macOS platforms)
#[cfg(not(target_os = "macos"))]
pub fn create_hard_link(source: &Path, dest: &Path) -> Result<(), Error> {
    std::fs::hard_link(source, dest).map_err(|e| {
        InstallError::FilesystemError {
            operation: "create_hard_link".to_string(),
            path: source.display().to_string(),
            message: e.to_string(),
        }
        .into()
    })
}

/// Create hard links recursively without tracking (for legacy code)
#[allow(dead_code)]
pub async fn create_hardlinks_recursive(source: &Path, dest_prefix: &Path) -> Result<(), Error> {
    let mut dummy_paths = Vec::new();
    create_hardlinks_recursive_with_tracking(source, dest_prefix, source, &mut dummy_paths).await
}

/// Create hard links recursively and track file paths
pub async fn create_hardlinks_recursive_with_tracking(
    source: &Path,
    dest_prefix: &Path,
    root_source: &Path,
    file_paths: &mut Vec<(String, bool)>,
) -> Result<(), Error> {
    let mut entries = fs::read_dir(source).await?;

    while let Some(entry) = entries.next_entry().await? {
        let entry_path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dest_prefix.join(&file_name);

        // Calculate relative path from store root
        let relative_path =
            entry_path
                .strip_prefix(root_source)
                .map_err(|e| InstallError::FilesystemError {
                    operation: "calculate_relative_path".to_string(),
                    path: entry_path.display().to_string(),
                    message: e.to_string(),
                })?;

        if entry_path.is_dir() {
            // Create directory and recurse
            fs::create_dir_all(&dest_path).await?;

            // Record directory in file tracking
            file_paths.push((relative_path.display().to_string(), true));

            Box::pin(create_hardlinks_recursive_with_tracking(
                &entry_path,
                &dest_path,
                root_source,
                file_paths,
            ))
            .await?;
        } else {
            // Create hard link only if destination doesn't already exist
            if !dest_path.exists() {
                #[cfg(target_os = "macos")]
                {
                    // Use APFS hard link on macOS
                    create_hard_link(&entry_path, &dest_path)?;
                }

                #[cfg(not(target_os = "macos"))]
                {
                    // Use standard hard link on other platforms
                    create_hard_link(&entry_path, &dest_path)?;
                }
            }

            // Record file in file tracking (whether newly linked or already existed)
            file_paths.push((relative_path.display().to_string(), false));
        }
    }

    Ok(())
}

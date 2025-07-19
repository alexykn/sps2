//! Deterministic TAR archive creation for reproducible builds

use sps2_errors::{BuildError, Error};
use std::path::{Path, PathBuf};
use tokio::fs::File;

/// Default deterministic timestamp (Unix epoch) for reproducible builds
const DETERMINISTIC_TIMESTAMP: u64 = 0;

/// Environment variable for `SOURCE_DATE_EPOCH` (standard for reproducible builds)
const SOURCE_DATE_EPOCH_VAR: &str = "SOURCE_DATE_EPOCH";

/// Create deterministic tar archive from directory using the tar crate
/// Ensures identical input produces identical compressed output for reproducible builds
/// Create a deterministic tar archive from a source directory
///
/// # Errors
///
/// Returns an error if file I/O operations fail or tar creation fails.
pub async fn create_deterministic_tar_archive(
    source_dir: &Path,
    tar_path: &Path,
) -> Result<(), Error> {
    // Use the global deterministic timestamp
    let deterministic_timestamp = get_deterministic_timestamp();
    create_deterministic_tar_archive_with_timestamp(source_dir, tar_path, deterministic_timestamp)
        .await
}

/// Create deterministic tar archive with explicit timestamp (for testing)
/// Ensures identical input produces identical compressed output for reproducible builds
pub async fn create_deterministic_tar_archive_with_timestamp(
    source_dir: &Path,
    tar_path: &Path,
    timestamp: u64,
) -> Result<(), Error> {
    use tar::Builder;

    let file = File::create(tar_path).await?;
    let file = file.into_std().await;
    let source_dir = source_dir.to_path_buf(); // Clone to move into closure

    // Create deterministic tar using the tar crate
    tokio::task::spawn_blocking(move || -> Result<(), Error> {
        let mut tar_builder = Builder::new(file);

        // Set deterministic behavior
        tar_builder.follow_symlinks(false);

        add_directory_to_tar_with_timestamp(&mut tar_builder, &source_dir, "".as_ref(), timestamp)?;
        tar_builder.finish()?;

        Ok(())
    })
    .await
    .map_err(|e| BuildError::Failed {
        message: format!("tar creation task failed: {e}"),
    })??;

    Ok(())
}

/// Recursively add directory contents to tar archive with deterministic ordering
/// This is the enhanced deterministic version with improved file ordering and metadata normalization
/// for reproducible builds
fn add_directory_to_tar_with_timestamp(
    tar_builder: &mut tar::Builder<std::fs::File>,
    dir_path: &Path,
    tar_path: &Path,
    deterministic_timestamp: u64,
) -> Result<(), Error> {
    let mut entries = std::fs::read_dir(dir_path)?.collect::<Result<Vec<_>, _>>()?;

    // Enhanced deterministic sorting for optimal compression:
    // 1. Sort all entries lexicographically by filename (case-sensitive, locale-independent)
    // 2. This ensures consistent ordering across different filesystems and locales
    entries.sort_by(|a, b| {
        // Use OS string comparison for consistent, locale-independent ordering
        a.file_name().cmp(&b.file_name())
    });

    for entry in entries {
        let file_path = entry.path();
        let file_name = entry.file_name();

        // Skip the package.tar file if it exists to avoid recursion
        if file_name == "package.tar" {
            continue;
        }

        // Construct tar entry path - avoid leading separators for root entries
        let tar_entry_path = if tar_path.as_os_str().is_empty() {
            PathBuf::from(&file_name)
        } else {
            tar_path.join(&file_name)
        };

        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            // Add directory entry with fully normalized metadata
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(normalize_file_permissions(&metadata));
            header.set_mtime(deterministic_timestamp);
            header.set_uid(0); // Normalized ownership
            header.set_gid(0); // Normalized ownership
            header.set_username("root")?; // Consistent username
            header.set_groupname("root")?; // Consistent group name
            header.set_device_major(0)?; // Clear device numbers
            header.set_device_minor(0)?; // Clear device numbers
            header.set_cksum();

            let tar_dir_path = format!("{}/", tar_entry_path.display());
            tar_builder.append_data(&mut header, &tar_dir_path, std::io::empty())?;

            // Recursively add directory contents
            add_directory_to_tar_with_timestamp(
                tar_builder,
                &file_path,
                &tar_entry_path,
                deterministic_timestamp,
            )?;
        } else if metadata.is_file() {
            // Add file entry with fully normalized metadata
            let mut file = std::fs::File::open(&file_path)?;
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(metadata.len());
            header.set_mode(normalize_file_permissions(&metadata));
            header.set_mtime(deterministic_timestamp);
            header.set_uid(0); // Normalized ownership
            header.set_gid(0); // Normalized ownership
            header.set_username("root")?; // Consistent username
            header.set_groupname("root")?; // Consistent group name
            header.set_device_major(0)?; // Clear device numbers
            header.set_device_minor(0)?; // Clear device numbers
            header.set_cksum();

            tar_builder.append_data(
                &mut header,
                tar_entry_path.display().to_string(),
                &mut file,
            )?;
        } else if metadata.is_symlink() {
            // Handle symlinks deterministically
            let target = std::fs::read_link(&file_path)?;
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777); // Standard symlink permissions
            header.set_mtime(deterministic_timestamp);
            header.set_uid(0); // Normalized ownership
            header.set_gid(0); // Normalized ownership
            header.set_username("root")?; // Consistent username
            header.set_groupname("root")?; // Consistent group name
            header.set_link_name(&target)?;
            header.set_device_major(0)?; // Clear device numbers
            header.set_device_minor(0)?; // Clear device numbers
            header.set_cksum();

            tar_builder.append_data(
                &mut header,
                tar_entry_path.display().to_string(),
                std::io::empty(),
            )?;
        }
        // Skip other special files (device nodes, fifos, etc.) for security and consistency
    }

    Ok(())
}

/// Get deterministic timestamp for reproducible builds
/// Uses `SOURCE_DATE_EPOCH` if set, otherwise uses epoch (0)
#[must_use]
pub fn get_deterministic_timestamp() -> u64 {
    std::env::var(SOURCE_DATE_EPOCH_VAR)
        .ok()
        .and_then(|val| val.parse::<u64>().ok())
        .unwrap_or(DETERMINISTIC_TIMESTAMP)
}

/// Normalize file permissions for deterministic output
/// Ensures consistent permissions across different filesystems and umask settings
fn normalize_file_permissions(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    let current_mode = metadata.permissions().mode();

    if metadata.is_dir() {
        0o755 // Directories: rwxr-xr-x
    } else if metadata.is_file() {
        // Files: check if any execute bit is set
        if current_mode & 0o111 != 0 {
            0o755 // Executable files: rwxr-xr-x
        } else {
            0o644 // Regular files: rw-r--r--
        }
    } else {
        0o644 // Default for other file types
    }
}

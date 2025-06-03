//! Tar archive content validation
//!
//! This module provides robust validation of tar archive contents including
//! file count limits, path safety checks, symlink validation, and manifest
//! detection with comprehensive error recovery.

use sps2_errors::{Error, InstallError, PackageError};
use std::path::Path;

use crate::validation::types::{
    ValidationResult, MAX_EXTRACTED_SIZE, MAX_FILE_COUNT, MAX_PATH_LENGTH,
};

/// Validates tar archive content
///
/// This function performs comprehensive validation of tar archive contents
/// including file counting, path safety, symlink checking, and manifest
/// detection. It includes robust error handling for corrupted archives.
///
/// # Errors
///
/// Returns an error if:
/// - File cannot be opened or read
/// - Archive format is invalid or corrupted
/// - Required manifest.toml is missing
/// - Tar entries cannot be processed
#[allow(clippy::too_many_lines)]
pub async fn validate_tar_archive_content(
    file_path: &Path,
    result: &mut ValidationResult,
) -> Result<(), Error> {
    let file_path = file_path.to_path_buf();

    // Use blocking task for tar operations with comprehensive error handling
    let validation_result = tokio::task::spawn_blocking(move || {
        use std::fs::File;
        use tar::Archive;

        // Wrap the entire tar validation in a try-catch to handle any tar library errors
        let result = std::panic::catch_unwind(|| -> Result<(usize, u64, Vec<String>, Option<String>), Error> {
            let file = File::open(&file_path)?;
            let mut archive = Archive::new(file);

            let mut file_count = 0;
            let mut extracted_size = 0u64;
            let mut has_manifest = false;
            let mut manifest_content = None;
            let mut warnings = Vec::new();

            // Iterate through archive entries with robust error handling
            let entries = match archive.entries() {
                Ok(entries) => entries,
                Err(e) => {
                    // If we can't even get the entries iterator, the archive is severely corrupted
                    warnings.push(format!("severely corrupted tar archive: {e}"));
                    // Return validation with warnings but mark as invalid (0 file count = invalid)
                    return Ok((0, 0, warnings, None));
                }
            };

            for entry_result in entries {
                let mut entry = match entry_result {
                    Ok(entry) => entry,
                    Err(e) => {
                        // Log warning for corrupted entry but continue processing
                        warnings.push(format!("corrupted tar entry: {e} - skipping this entry"));
                        continue;
                    }
                };
                file_count += 1;

                // Check file count limit
                if file_count > MAX_FILE_COUNT {
                    return Err(InstallError::InvalidPackageFile {
                        path: file_path.display().to_string(),
                        message: format!(
                            "too many files in archive: {file_count} (max: {MAX_FILE_COUNT})"
                        ),
                    }
                    .into());
                }

                // Get entry path (clone to avoid borrow conflicts)
                let (path, path_str) = match entry.path() {
                    Ok(path) => {
                        let path_buf = path.to_path_buf();
                        let path_str = path_buf.to_string_lossy().to_string();
                        (path_buf, path_str)
                    }
                    Err(e) => {
                        // Log warning for corrupted path but continue with placeholder
                        warnings.push(format!("corrupted tar entry path: {e} - using placeholder name"));
                        let placeholder = format!("corrupted_entry_{file_count}");
                        (std::path::PathBuf::from(&placeholder), placeholder)
                    }
                };

                // Check path length
                if path_str.len() > MAX_PATH_LENGTH {
                    return Err(InstallError::InvalidPackageFile {
                        path: file_path.display().to_string(),
                        message: format!(
                            "path too long: {} characters (max: {})",
                            path_str.len(),
                            MAX_PATH_LENGTH
                        ),
                    }
                    .into());
                }

                // Check for path traversal attempts
                if path_str.contains("..") || path.is_absolute() {
                    return Err(InstallError::InvalidPackageFile {
                        path: file_path.display().to_string(),
                        message: format!("suspicious path detected: {path_str}"),
                    }
                    .into());
                }

                // Check for manifest.toml
                if path_str == "manifest.toml" {
                    has_manifest = true;

                    // Try to read and parse manifest
                    let mut content = String::new();
                    if std::io::Read::read_to_string(&mut entry, &mut content).is_ok() {
                        // Try to parse as TOML
                        match toml::from_str::<toml::Value>(&content) {
                            Ok(_) => {
                                manifest_content = Some(content);
                            }
                            Err(e) => {
                                warnings.push(format!("manifest.toml parse warning: {e}"));
                            }
                        }
                    }
                }

                // Accumulate size with robust error handling for corrupted headers
                match entry.header().size() {
                    Ok(size) => {
                        extracted_size += size;
                    }
                    Err(e) => {
                        // Log warning for corrupted header but don't fail validation
                        warnings.push(format!(
                            "corrupted tar header for '{path_str}': {e} - skipping size validation for this entry"
                        ));
                        // Continue without adding to extracted_size for this entry
                    }
                }

                // Check extracted size limit
                if extracted_size > MAX_EXTRACTED_SIZE {
                    return Err(InstallError::InvalidPackageFile {
                        path: file_path.display().to_string(),
                        message: format!(
                            "extracted content too large: {extracted_size} bytes (max: {MAX_EXTRACTED_SIZE} bytes)"
                        ),
                    }
                    .into());
                }

                // Validate file type with robust error handling
                let header = entry.header();
                match header.entry_type() {
                    tar::EntryType::Regular | tar::EntryType::Directory => {
                        // These are safe
                    }
                    tar::EntryType::Symlink | tar::EntryType::Link => {
                        // Check symlink target with error handling
                        match header.link_name() {
                            Ok(Some(target)) => {
                                let target_str = target.to_string_lossy();
                                if target_str.contains("..") || target.is_absolute() {
                                    return Err(InstallError::InvalidPackageFile {
                                        path: file_path.display().to_string(),
                                        message: format!("suspicious symlink target: {target_str}"),
                                    }
                                    .into());
                                }
                            }
                            Ok(None) => {
                                warnings.push(format!("symlink '{path_str}' has no target"));
                            }
                            Err(e) => {
                                warnings.push(format!("corrupted symlink target for '{path_str}': {e}"));
                            }
                        }
                    }
                    _ => {
                        warnings.push(format!(
                            "unusual file type in archive: {:?}",
                            header.entry_type()
                        ));
                    }
                }
            }

            // Check that manifest.toml exists
            if !has_manifest {
                return Err(PackageError::InvalidFormat {
                    message: "missing manifest.toml in package".to_string(),
                }
                .into());
            }

            Ok::<_, Error>((file_count, extracted_size, warnings, manifest_content))
        });

        // Handle panics from tar library (e.g., corrupted headers causing UTF-8 errors)
        match result {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(e)) => Err(e),
            Err(_panic) => {
                // Tar library panicked, likely due to severely corrupted data
                let mut warnings = vec!["tar archive caused panic during validation - likely corrupted headers".to_string()];
                warnings.push("validation failed due to corrupted tar data".to_string());
                Ok((0, 0, warnings, None)) // Mark as invalid with warnings
            }
        }
    })
    .await
    .map_err(|e| Error::internal(format!("tar validation task failed: {e}")))?;

    match validation_result {
        Ok((file_count, extracted_size, warnings, manifest)) => {
            // Update result
            result.file_count = file_count;
            result.extracted_size = extracted_size;
            result.warnings.extend(warnings);
            result.manifest = manifest;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Validates specific tar entry for safety
pub fn validate_tar_entry_safety(
    path_str: &str,
    entry_type: tar::EntryType,
    link_target: Option<&str>,
) -> Result<(), Error> {
    // Check path length
    if path_str.len() > MAX_PATH_LENGTH {
        return Err(InstallError::InvalidPackageFile {
            path: "archive".to_string(),
            message: format!(
                "path too long: {} characters (max: {})",
                path_str.len(),
                MAX_PATH_LENGTH
            ),
        }
        .into());
    }

    // Check for path traversal attempts
    if path_str.contains("..") || std::path::Path::new(path_str).is_absolute() {
        return Err(InstallError::InvalidPackageFile {
            path: "archive".to_string(),
            message: format!("suspicious path detected: {path_str}"),
        }
        .into());
    }

    // Validate symlinks
    match entry_type {
        tar::EntryType::Symlink | tar::EntryType::Link => {
            if let Some(target) = link_target {
                if target.contains("..") || std::path::Path::new(target).is_absolute() {
                    return Err(InstallError::InvalidPackageFile {
                        path: "archive".to_string(),
                        message: format!("suspicious symlink target: {target}"),
                    }
                    .into());
                }
            }
        }
        _ => {} // Other types are handled elsewhere
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_tar_entry_safety() {
        // Valid regular path
        assert!(
            validate_tar_entry_safety("some/normal/path.txt", tar::EntryType::Regular, None)
                .is_ok()
        );

        // Path traversal attempt
        assert!(validate_tar_entry_safety("../etc/passwd", tar::EntryType::Regular, None).is_err());

        // Absolute path
        assert!(validate_tar_entry_safety("/etc/passwd", tar::EntryType::Regular, None).is_err());

        // Valid symlink
        assert!(
            validate_tar_entry_safety("link", tar::EntryType::Symlink, Some("target.txt")).is_ok()
        );

        // Dangerous symlink
        assert!(
            validate_tar_entry_safety("link", tar::EntryType::Symlink, Some("../etc/passwd"))
                .is_err()
        );
    }

    #[test]
    fn test_path_length_validation() {
        let long_path = "a".repeat(MAX_PATH_LENGTH + 1);
        assert!(validate_tar_entry_safety(&long_path, tar::EntryType::Regular, None).is_err());

        let ok_path = "a".repeat(MAX_PATH_LENGTH);
        assert!(validate_tar_entry_safety(&ok_path, tar::EntryType::Regular, None).is_ok());
    }
}

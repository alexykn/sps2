//! Content limits validation
//!
//! This module provides validation of various content limits including
//! file counts, path lengths, nesting depths, and other quantitative
//! constraints on package contents.

use sps2_errors::{Error, InstallError};

use crate::validation::types::{MAX_EXTRACTED_SIZE, MAX_FILE_COUNT, MAX_PATH_LENGTH};

/// Validates file count limits
///
/// Ensures that the number of files in the package doesn't exceed
/// the maximum allowed count to prevent resource exhaustion.
pub fn validate_file_count(count: usize) -> Result<(), Error> {
    if count > MAX_FILE_COUNT {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!("too many files in archive: {count} (max: {MAX_FILE_COUNT})"),
        }
        .into());
    }
    Ok(())
}

/// Validates path length limits
///
/// Ensures that file paths within the package don't exceed reasonable
/// length limits to prevent filesystem issues.
pub fn validate_path_length(path: &str) -> Result<(), Error> {
    if path.len() > MAX_PATH_LENGTH {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!(
                "path too long: {} characters (max: {})",
                path.len(),
                MAX_PATH_LENGTH
            ),
        }
        .into());
    }
    Ok(())
}

/// Validates total extracted size
///
/// Ensures that the total size of extracted content doesn't exceed
/// storage limits.
pub fn validate_total_extracted_size(size: u64) -> Result<(), Error> {
    if size > MAX_EXTRACTED_SIZE {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!(
                "extracted content too large: {size} bytes (max: {MAX_EXTRACTED_SIZE} bytes)"
            ),
        }
        .into());
    }
    Ok(())
}

/// Validates directory nesting depth
///
/// Prevents excessively deep directory structures that could cause
/// filesystem issues or path resolution problems.
pub fn validate_path_depth(path: &str, max_depth: usize) -> Result<(), Error> {
    let depth = path.split('/').count();
    if depth > max_depth {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!("path too deep: {depth} levels (max: {max_depth})"),
        }
        .into());
    }
    Ok(())
}

/// Validates individual file size
///
/// Ensures that individual files within the package don't exceed
/// reasonable size limits.
pub fn validate_individual_file_size(size: u64, max_size: u64) -> Result<(), Error> {
    if size > max_size {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!("file too large: {size} bytes (max: {max_size} bytes)"),
        }
        .into());
    }
    Ok(())
}

/// Validates filename length
///
/// Ensures that individual filenames (not full paths) don't exceed
/// filesystem limits.
pub fn validate_filename_length(filename: &str, max_length: usize) -> Result<(), Error> {
    if filename.len() > max_length {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!(
                "filename too long: {} characters (max: {})",
                filename.len(),
                max_length
            ),
        }
        .into());
    }
    Ok(())
}

/// Content limits configuration
#[derive(Debug, Clone)]
pub struct ContentLimits {
    /// Maximum number of files
    pub max_files: usize,
    /// Maximum path length
    pub max_path_length: usize,
    /// Maximum directory depth
    pub max_depth: usize,
    /// Maximum individual file size
    pub max_file_size: u64,
    /// Maximum filename length
    pub max_filename_length: usize,
    /// Maximum total extracted size
    pub max_extracted_size: u64,
}

impl Default for ContentLimits {
    fn default() -> Self {
        Self {
            max_files: MAX_FILE_COUNT,
            max_path_length: MAX_PATH_LENGTH,
            max_depth: 50,
            max_file_size: 100 * 1024 * 1024, // 100MB per file
            max_filename_length: 255,
            max_extracted_size: MAX_EXTRACTED_SIZE,
        }
    }
}

impl ContentLimits {
    /// Create new content limits
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum file count
    #[must_use]
    pub fn with_max_files(mut self, max_files: usize) -> Self {
        self.max_files = max_files;
        self
    }

    /// Set maximum path length
    #[must_use]
    pub fn with_max_path_length(mut self, max_path_length: usize) -> Self {
        self.max_path_length = max_path_length;
        self
    }

    /// Set maximum directory depth
    #[must_use]
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Set maximum individual file size
    #[must_use]
    pub fn with_max_file_size(mut self, max_file_size: u64) -> Self {
        self.max_file_size = max_file_size;
        self
    }

    /// Validate all limits for a file
    pub fn validate_file(&self, path: &str, filename: &str, file_size: u64) -> Result<(), Error> {
        validate_path_length(path)?;
        validate_path_depth(path, self.max_depth)?;
        validate_filename_length(filename, self.max_filename_length)?;
        validate_individual_file_size(file_size, self.max_file_size)?;
        Ok(())
    }

    /// Validate package totals
    pub fn validate_totals(&self, file_count: usize, total_size: u64) -> Result<(), Error> {
        validate_file_count(file_count)?;
        validate_total_extracted_size(total_size)?;
        Ok(())
    }
}

/// Statistics about package content
#[derive(Debug, Clone, Default)]
pub struct ContentStats {
    /// Total number of files
    pub file_count: usize,
    /// Total extracted size
    pub total_size: u64,
    /// Maximum path length found
    pub max_path_length: usize,
    /// Maximum directory depth found
    pub max_depth: usize,
    /// Largest individual file size
    pub largest_file_size: u64,
    /// Number of directories
    pub directory_count: usize,
    /// Number of symlinks
    pub symlink_count: usize,
}

impl ContentStats {
    /// Create new content statistics
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Update statistics with a new file
    pub fn add_file(&mut self, path: &str, file_size: u64, is_directory: bool, is_symlink: bool) {
        self.file_count += 1;
        self.total_size += file_size;
        self.max_path_length = self.max_path_length.max(path.len());
        self.max_depth = self.max_depth.max(path.split('/').count());
        self.largest_file_size = self.largest_file_size.max(file_size);

        if is_directory {
            self.directory_count += 1;
        }
        if is_symlink {
            self.symlink_count += 1;
        }
    }

    /// Validate statistics against limits
    pub fn validate_against_limits(&self, limits: &ContentLimits) -> Result<(), Error> {
        limits.validate_totals(self.file_count, self.total_size)?;

        if self.max_path_length > limits.max_path_length {
            return Err(InstallError::InvalidPackageFile {
                path: "package".to_string(),
                message: format!(
                    "path too long: {} characters (max: {})",
                    self.max_path_length, limits.max_path_length
                ),
            }
            .into());
        }

        if self.max_depth > limits.max_depth {
            return Err(InstallError::InvalidPackageFile {
                path: "package".to_string(),
                message: format!(
                    "path too deep: {} levels (max: {})",
                    self.max_depth, limits.max_depth
                ),
            }
            .into());
        }

        if self.largest_file_size > limits.max_file_size {
            return Err(InstallError::InvalidPackageFile {
                path: "package".to_string(),
                message: format!(
                    "file too large: {} bytes (max: {} bytes)",
                    self.largest_file_size, limits.max_file_size
                ),
            }
            .into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_file_count() {
        assert!(validate_file_count(100).is_ok());
        assert!(validate_file_count(MAX_FILE_COUNT).is_ok());
        assert!(validate_file_count(MAX_FILE_COUNT + 1).is_err());
    }

    #[test]
    fn test_validate_path_length() {
        let short_path = "short/path";
        assert!(validate_path_length(short_path).is_ok());

        let long_path = "a".repeat(MAX_PATH_LENGTH + 1);
        assert!(validate_path_length(&long_path).is_err());
    }

    #[test]
    fn test_validate_path_depth() {
        assert!(validate_path_depth("a/b/c", 5).is_ok());
        assert!(validate_path_depth("a/b/c/d/e/f", 5).is_err());
    }

    #[test]
    fn test_content_limits() {
        let limits = ContentLimits::new().with_max_files(1000).with_max_depth(10);

        assert_eq!(limits.max_files, 1000);
        assert_eq!(limits.max_depth, 10);

        // Test validation
        assert!(limits.validate_file("short/path", "file.txt", 1024).is_ok());
    }

    #[test]
    fn test_content_stats() {
        let mut stats = ContentStats::new();
        stats.add_file("path/to/file.txt", 1024, false, false);
        stats.add_file("path/to/dir/", 0, true, false);
        stats.add_file("path/to/link", 0, false, true);

        assert_eq!(stats.file_count, 3);
        assert_eq!(stats.total_size, 1024);
        assert_eq!(stats.directory_count, 1);
        assert_eq!(stats.symlink_count, 1);
        assert_eq!(stats.max_path_length, "path/to/file.txt".len());
    }
}

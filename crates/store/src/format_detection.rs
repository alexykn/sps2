//! Package format version detection and validation for store operations
//!
//! This module provides fast format version detection without full package parsing,
//! enabling compatibility checking and migration support.

use sps2_errors::{Error, PackageError, StorageError};
use sps2_types::{PackageFormatChecker, PackageFormatValidationResult, PackageFormatVersion};
use std::path::Path;
use tokio::{fs::File, io::AsyncReadExt};

/// Package format detection result
#[derive(Debug, Clone)]
pub struct PackageFormatInfo {
    /// Detected format version
    pub version: PackageFormatVersion,
    /// Whether fast header detection was used
    pub from_header: bool,
    /// Whether format is compatible with current version
    pub is_compatible: bool,
    /// Validation result with details
    pub validation: PackageFormatValidationResult,
}

/// Package format detector for .sp files
#[derive(Clone, Debug)]
pub struct PackageFormatDetector {
    checker: PackageFormatChecker,
}

impl PackageFormatDetector {
    /// Create a new format detector
    #[must_use]
    pub fn new() -> Self {
        Self {
            checker: PackageFormatChecker::new(),
        }
    }

    /// Detect package format version from .sp file
    ///
    /// This method first attempts fast header detection, then falls back to
    /// manifest parsing if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - File cannot be read
    /// - Package format is invalid or corrupted
    /// - I/O operations fail
    pub async fn detect_format(&self, sp_file: &Path) -> Result<PackageFormatInfo, Error> {
        // First try fast header detection
        if let Ok(version) = self.detect_from_header(sp_file).await {
            let validation = self.checker.validate_version(&version);
            let is_compatible = matches!(
                validation,
                PackageFormatValidationResult::Compatible
                    | PackageFormatValidationResult::BackwardsCompatible { .. }
            );

            return Ok(PackageFormatInfo {
                version,
                from_header: true,
                is_compatible,
                validation,
            });
        }

        // Fall back to manifest parsing
        let version = self.detect_from_manifest(sp_file).await?;
        let validation = self.checker.validate_version(&version);
        let is_compatible = matches!(
            validation,
            PackageFormatValidationResult::Compatible
                | PackageFormatValidationResult::BackwardsCompatible { .. }
        );

        Ok(PackageFormatInfo {
            version,
            from_header: false,
            is_compatible,
            validation,
        })
    }

    /// Fast format version detection from package header
    ///
    /// This method reads only the first few bytes of the package to detect
    /// the format version without decompressing or parsing the entire package.
    ///
    /// # Errors
    ///
    /// Returns an error if the header cannot be read or is invalid
    pub async fn detect_from_header(&self, sp_file: &Path) -> Result<PackageFormatVersion, Error> {
        let mut file = File::open(sp_file)
            .await
            .map_err(|e| StorageError::IoError {
                message: format!("failed to open package for header detection: {e}"),
            })?;

        // Read enough bytes for both zstd header and version header
        let mut header_buffer = vec![0u8; 64]; // 64 bytes should be enough
        let bytes_read =
            file.read(&mut header_buffer)
                .await
                .map_err(|e| StorageError::IoError {
                    message: format!("failed to read package header: {e}"),
                })?;

        header_buffer.truncate(bytes_read);

        // Look for version header pattern after zstd header
        Self::find_version_header_in_buffer(&header_buffer)
    }

    /// Detect format version from manifest inside the package
    ///
    /// This method extracts and parses the manifest.toml to get the format version.
    /// It's slower but more reliable than header detection.
    ///
    /// # Errors
    ///
    /// Returns an error if manifest extraction or parsing fails
    pub async fn detect_from_manifest(
        &self,
        sp_file: &Path,
    ) -> Result<PackageFormatVersion, Error> {
        // Create temporary directory for manifest extraction
        let temp_dir = tempfile::tempdir().map_err(|e| StorageError::IoError {
            message: format!("failed to create temp dir for manifest extraction: {e}"),
        })?;

        // Extract the package (full extraction since partial extraction was removed)
        crate::archive::extract_package(sp_file, temp_dir.path()).await?;

        // Read and parse the manifest
        let manifest_path = temp_dir.path().join("manifest.toml");
        let manifest = crate::manifest_io::read_manifest(&manifest_path).await?;

        Ok(manifest.format_version().clone())
    }

    /// Find version header pattern in a buffer
    ///
    /// Looks for the version header magic bytes (SPV1) and extracts version information
    fn find_version_header_in_buffer(buffer: &[u8]) -> Result<PackageFormatVersion, Error> {
        const VERSION_MAGIC: [u8; 4] = [0x53, 0x50, 0x56, 0x31]; // "SPV1"

        // Search for the version magic bytes in the buffer
        for window_start in 0..buffer.len().saturating_sub(12) {
            let window = &buffer[window_start..window_start + 12];

            if window.len() >= 12 && window[0..4] == VERSION_MAGIC {
                // Found version header, parse it
                return PackageFormatVersion::from_header_bytes(window).map_err(|e| {
                    PackageError::InvalidFormat {
                        message: format!("failed to parse version header: {e}"),
                    }
                    .into()
                });
            }
        }

        Err(PackageError::InvalidFormat {
            message: "No version header found in package".to_string(),
        }
        .into())
    }

    /// Validate package format compatibility
    ///
    /// Checks if a package with the given format version can be processed
    /// by the current version of sps2.
    ///
    /// # Errors
    ///
    /// Returns an error if the package format is incompatible
    pub fn validate_compatibility(&self, format_info: &PackageFormatInfo) -> Result<(), Error> {
        match &format_info.validation {
            PackageFormatValidationResult::Compatible => Ok(()),
            PackageFormatValidationResult::BackwardsCompatible { warning: _ } => {
                // Allow processing without direct printing; callers may emit events if needed
                Ok(())
            }
            PackageFormatValidationResult::RequiresMigration { migration: _ } => {
                Err(PackageError::IncompatibleFormat {
                    version: format_info.version.to_string(),
                    reason: "Package requires migration to current format".to_string(),
                }
                .into())
            }
            PackageFormatValidationResult::Incompatible { reason, suggestion } => {
                Err(PackageError::IncompatibleFormat {
                    version: format_info.version.to_string(),
                    reason: format!("{reason}. {suggestion}"),
                }
                .into())
            }
        }
    }

    /// Check if a package supports a specific feature based on its format version
    #[must_use]
    pub fn supports_feature(&self, version: &PackageFormatVersion, feature: &str) -> bool {
        let compat_info = version.compatibility_info();

        match feature {
            "signatures" => compat_info.supports_signatures,
            "seekable_compression" => true,
            _ => false,
        }
    }
}

impl Default for PackageFormatDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Store-level format validation for package operations
#[derive(Clone, Debug)]
pub struct StoreFormatValidator {
    detector: PackageFormatDetector,
    require_compatibility: bool,
}

impl StoreFormatValidator {
    /// Create a new store format validator
    #[must_use]
    pub fn new() -> Self {
        Self {
            detector: PackageFormatDetector::new(),
            require_compatibility: true,
        }
    }

    /// Create a validator that allows incompatible packages (for migration tools)
    #[must_use]
    pub fn allow_incompatible() -> Self {
        Self {
            detector: PackageFormatDetector::new(),
            require_compatibility: false,
        }
    }

    /// Validate package format before store operations
    ///
    /// # Errors
    ///
    /// Returns an error if package format is incompatible and compatibility is required
    pub async fn validate_before_storage(
        &self,
        sp_file: &Path,
    ) -> Result<PackageFormatInfo, Error> {
        let format_info = self.detector.detect_format(sp_file).await?;

        if self.require_compatibility {
            self.detector.validate_compatibility(&format_info)?;
        }

        Ok(format_info)
    }

    /// Validate package format after loading from store
    ///
    /// # Errors
    ///
    /// Returns an error if manifest parsing fails or format is incompatible
    pub async fn validate_stored_package(
        &self,
        package_path: &Path,
    ) -> Result<PackageFormatInfo, Error> {
        // For stored packages, read the manifest directly
        let manifest_path = package_path.join("manifest.toml");
        let manifest = crate::manifest_io::read_manifest(&manifest_path).await?;

        let version = manifest.format_version().clone();
        let validation = self.detector.checker.validate_version(&version);
        let is_compatible = matches!(
            validation,
            PackageFormatValidationResult::Compatible
                | PackageFormatValidationResult::BackwardsCompatible { .. }
        );

        let format_info = PackageFormatInfo {
            version,
            from_header: false, // Read from manifest
            is_compatible,
            validation,
        };

        if self.require_compatibility {
            self.detector.validate_compatibility(&format_info)?;
        }

        Ok(format_info)
    }
}

impl Default for StoreFormatValidator {
    fn default() -> Self {
        Self::new()
    }
}

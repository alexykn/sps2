//! Manifest validation
//!
//! This module provides validation of package manifest.toml files using
//! the proper manifest structure defined in the manifest crate.

use sps2_errors::Error;
use sps2_types::Manifest;

/// Validates manifest.toml content
///
/// This function parses the manifest using the proper Manifest type
/// and validates that it contains all required fields with valid values.
///
/// # Errors
///
/// Returns an error if:
/// - TOML syntax is invalid
/// - Required fields are missing
/// - Field values are invalid
pub fn validate_manifest_content(content: &str) -> Result<ManifestValidation, Error> {
    let mut validation = ManifestValidation::new();

    // Parse using the proper Manifest type
    match Manifest::from_toml(content) {
        Ok(manifest) => {
            // Validate the manifest using its built-in validation
            match manifest.validate() {
                Ok(()) => {
                    validation.mark_field_valid("package");
                    validation.mark_field_valid("dependencies");
                    validation.mark_field_valid("format_version");
                }
                Err(e) => {
                    validation.add_error(format!("manifest validation failed: {e}"));
                }
            }

            // Add warnings for missing optional fields
            if manifest.package.description.is_none() {
                validation
                    .add_warning("missing recommended field: package.description".to_string());
            }
            if manifest.package.homepage.is_none() {
                validation.add_warning("missing recommended field: package.homepage".to_string());
            }
            if manifest.package.license.is_none() {
                validation.add_warning("missing recommended field: package.license".to_string());
            }
        }
        Err(e) => {
            validation.add_error(format!("failed to parse manifest: {e}"));
        }
    }

    if validation.has_errors() {
        Err(sps2_errors::PackageError::InvalidFormat {
            message: format!(
                "manifest validation failed: {}",
                validation.errors.join(", ")
            ),
        }
        .into())
    } else {
        Ok(validation)
    }
}

/// Result of manifest validation
#[derive(Debug)]
pub struct ManifestValidation {
    /// Validation errors (fatal)
    pub errors: Vec<String>,
    /// Validation warnings (non-fatal)
    pub warnings: Vec<String>,
    /// Valid fields found
    pub valid_fields: Vec<String>,
}

impl ManifestValidation {
    /// Create new validation result
    #[must_use]
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            valid_fields: Vec::new(),
        }
    }

    /// Add validation error
    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
    }

    /// Add validation warning
    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    /// Mark field as valid
    pub fn mark_field_valid(&mut self, field: &str) {
        self.valid_fields.push(field.to_string());
    }

    /// Check if validation has errors
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Check if validation is successful
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

impl Default for ManifestValidation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_manifest() {
        let manifest_content = r#"
[format_version]
major = 1
minor = 0
patch = 0

[package]
name = "test-package"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "A test package"
homepage = "https://example.com"
license = "MIT"

[dependencies]

[sbom]
spdx = "0734bf361c9b12811d7b63f6e116de06be7df63db29b9e9e25b13f98919236a2"
"#;

        let result = validate_manifest_content(manifest_content);
        assert!(
            result.is_ok(),
            "Valid manifest should pass validation: {:?}",
            result
        );

        let validation = result.unwrap();
        assert!(validation.is_valid(), "Validation should be successful");
        assert!(
            !validation.valid_fields.is_empty(),
            "Should have valid fields"
        );
    }

    #[test]
    fn test_real_package_manifest() {
        // This is the actual manifest from autoconf package
        let manifest_content = r#"
[format_version]
major = 1
minor = 0
patch = 0

[package]
name = "autoconf"
version = "2.72.0"
revision = 1
arch = "arm64"
description = "GNU Autoconf is a tool for producing shell scripts that automatically configure software source code packages."
homepage = "https://www.gnu.org/software/autoconf/"
license = "GPL-3.0-or-later"

[dependencies]

[sbom]
spdx = "0734bf361c9b12811d7b63f6e116de06be7df63db29b9e9e25b13f98919236a2"
"#;

        let result = validate_manifest_content(manifest_content);
        assert!(
            result.is_ok(),
            "Real package manifest should pass validation: {:?}",
            result
        );

        let validation = result.unwrap();
        assert!(
            validation.is_valid(),
            "Real package validation should be successful"
        );
    }

    #[test]
    fn test_invalid_manifest() {
        let manifest_content = r#"
[package]
name = ""
version = "invalid.version.format.too.many.parts"
arch = "unsupported"
"#;

        let result = validate_manifest_content(manifest_content);
        assert!(result.is_err(), "Invalid manifest should fail validation");
    }
}

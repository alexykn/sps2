//! Manifest validation
//!
//! This module provides validation of package manifest.toml files including
//! TOML syntax validation, required field checking, and semantic validation
//! of package metadata.

use sps2_errors::{Error, PackageError};

/// Required fields in a package manifest
const REQUIRED_FIELDS: &[&str] = &["name", "version", "description"];

/// Optional but recommended fields
const RECOMMENDED_FIELDS: &[&str] = &["license", "homepage", "authors"];

/// Validates manifest.toml content
///
/// This function parses the manifest as TOML and validates that it contains
/// all required fields with reasonable values.
///
/// # Errors
///
/// Returns an error if:
/// - TOML syntax is invalid
/// - Required fields are missing
/// - Field values are invalid
pub fn validate_manifest_content(content: &str) -> Result<ManifestValidation, Error> {
    // Parse as TOML
    let toml_value: toml::Value =
        toml::from_str(content).map_err(|e| PackageError::InvalidFormat {
            message: format!("manifest.toml syntax error: {e}"),
        })?;

    let mut validation = ManifestValidation::new();

    // Check for required fields
    for &field in REQUIRED_FIELDS {
        if let Some(value) = toml_value.get(field) {
            match validate_field(field, value) {
                Ok(()) => validation.mark_field_valid(field),
                Err(e) => validation.add_error(format!("{field}: {e}")),
            }
        } else {
            validation.add_error(format!("missing required field: {field}"));
        }
    }

    // Check for recommended fields
    for &field in RECOMMENDED_FIELDS {
        if toml_value.get(field).is_none() {
            validation.add_warning(format!("missing recommended field: {field}"));
        } else if let Some(value) = toml_value.get(field) {
            if let Err(e) = validate_field(field, value) {
                validation.add_warning(format!("{field}: {e}"));
            }
        }
    }

    // Validate dependencies if present
    if let Some(deps) = toml_value.get("dependencies") {
        if let Err(e) = validate_dependencies(deps) {
            validation.add_error(format!("dependencies: {e}"));
        }
    }

    // Validate build dependencies if present
    if let Some(build_deps) = toml_value.get("build_dependencies") {
        if let Err(e) = validate_dependencies(build_deps) {
            validation.add_error(format!("build_dependencies: {e}"));
        }
    }

    // Check for unknown fields (add as warnings)
    let known_fields = [
        "name",
        "version",
        "description",
        "license",
        "homepage",
        "authors",
        "dependencies",
        "build_dependencies",
        "files",
        "scripts",
        "metadata",
    ];

    if let toml::Value::Table(table) = &toml_value {
        for key in table.keys() {
            if !known_fields.contains(&key.as_str()) {
                validation.add_warning(format!("unknown field: {key}"));
            }
        }
    }

    if validation.has_errors() {
        Err(PackageError::InvalidFormat {
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

/// Validates a specific manifest field
fn validate_field(field: &str, value: &toml::Value) -> Result<(), String> {
    match field {
        "name" => validate_package_name(value),
        "version" => validate_version(value),
        "description" => validate_description(value),
        "license" => validate_license(value),
        "homepage" => validate_url(value),
        "authors" => validate_authors(value),
        _ => Ok(()), // Unknown fields are handled elsewhere
    }
}

/// Validates package name
fn validate_package_name(value: &toml::Value) -> Result<(), String> {
    let name = value.as_str().ok_or("must be a string")?;

    if name.is_empty() {
        return Err("cannot be empty".to_string());
    }

    if name.len() > 100 {
        return Err("too long (max 100 characters)".to_string());
    }

    // Check for valid characters (alphanumeric, dash, underscore)
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "can only contain alphanumeric characters, dashes, and underscores".to_string(),
        );
    }

    // Cannot start with dash
    if name.starts_with('-') {
        return Err("cannot start with dash".to_string());
    }

    Ok(())
}

/// Validates version string
fn validate_version(value: &toml::Value) -> Result<(), String> {
    let version = value.as_str().ok_or("must be a string")?;

    if version.is_empty() {
        return Err("cannot be empty".to_string());
    }

    // Basic semver validation (we could use a proper semver crate here)
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err("must be in format major.minor or major.minor.patch".to_string());
    }

    for part in &parts {
        if part.parse::<u32>().is_err() {
            return Err("version components must be numbers".to_string());
        }
    }

    Ok(())
}

/// Validates description
fn validate_description(value: &toml::Value) -> Result<(), String> {
    let description = value.as_str().ok_or("must be a string")?;

    if description.is_empty() {
        return Err("cannot be empty".to_string());
    }

    if description.len() > 500 {
        return Err("too long (max 500 characters)".to_string());
    }

    Ok(())
}

/// Validates license
fn validate_license(value: &toml::Value) -> Result<(), String> {
    let license = value.as_str().ok_or("must be a string")?;

    if license.is_empty() {
        return Err("cannot be empty".to_string());
    }

    // Common license identifiers
    let common_licenses = [
        "MIT",
        "Apache-2.0",
        "GPL-3.0",
        "BSD-3-Clause",
        "ISC",
        "MPL-2.0",
        "LGPL-3.0",
    ];

    if !common_licenses.contains(&license) {
        return Err(format!(
            "uncommon license '{}' (consider using SPDX identifier)",
            license
        ));
    }

    Ok(())
}

/// Validates URL
fn validate_url(value: &toml::Value) -> Result<(), String> {
    let url = value.as_str().ok_or("must be a string")?;

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("must be a valid HTTP(S) URL".to_string());
    }

    Ok(())
}

/// Validates authors array
fn validate_authors(value: &toml::Value) -> Result<(), String> {
    match value {
        toml::Value::Array(authors) => {
            if authors.is_empty() {
                return Err("cannot be empty array".to_string());
            }

            for author in authors {
                if let Some(author_str) = author.as_str() {
                    if author_str.is_empty() {
                        return Err("author cannot be empty string".to_string());
                    }
                } else {
                    return Err("all authors must be strings".to_string());
                }
            }

            Ok(())
        }
        toml::Value::String(author) => {
            if author.is_empty() {
                Err("cannot be empty string".to_string())
            } else {
                Ok(())
            }
        }
        _ => Err("must be string or array of strings".to_string()),
    }
}

/// Validates dependencies
fn validate_dependencies(value: &toml::Value) -> Result<(), String> {
    let deps = value.as_table().ok_or("must be a table")?;

    for (name, spec) in deps {
        // Validate dependency name
        if name.is_empty() {
            return Err("dependency name cannot be empty".to_string());
        }

        // Validate dependency specification
        match spec {
            toml::Value::String(version_spec) => {
                if version_spec.is_empty() {
                    return Err(format!("dependency '{name}' cannot have empty version"));
                }
                // Could add more sophisticated version spec validation here
            }
            toml::Value::Table(_) => {
                // Complex dependency spec - could validate further
            }
            _ => {
                return Err(format!(
                    "dependency '{name}' must have string or table specification"
                ));
            }
        }
    }

    Ok(())
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
    fn test_validate_manifest_valid() {
        let manifest = r#"
name = "test-package"
version = "1.0.0"
description = "A test package"
license = "MIT"

[dependencies]
curl = ">=8.0.0"
"#;

        let result = validate_manifest_content(manifest);
        assert!(result.is_ok());

        let validation = result.unwrap();
        assert!(validation.is_valid());
        assert!(validation.valid_fields.contains(&"name".to_string()));
    }

    #[test]
    fn test_validate_manifest_missing_required() {
        let manifest = r#"
name = "test-package"
# missing version and description
license = "MIT"
"#;

        let result = validate_manifest_content(manifest);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_manifest_invalid_syntax() {
        let manifest = "invalid toml [[[";

        let result = validate_manifest_content(manifest);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_package_name() {
        assert!(validate_package_name(&toml::Value::String("valid-name".to_string())).is_ok());
        assert!(validate_package_name(&toml::Value::String("valid_name".to_string())).is_ok());
        assert!(validate_package_name(&toml::Value::String("-invalid".to_string())).is_err());
        assert!(validate_package_name(&toml::Value::String("invalid!".to_string())).is_err());
        assert!(validate_package_name(&toml::Value::String(String::new())).is_err());
    }

    #[test]
    fn test_validate_version() {
        assert!(validate_version(&toml::Value::String("1.0.0".to_string())).is_ok());
        assert!(validate_version(&toml::Value::String("1.0".to_string())).is_ok());
        assert!(validate_version(&toml::Value::String("1".to_string())).is_err());
        assert!(validate_version(&toml::Value::String("1.0.0.0".to_string())).is_err());
        assert!(validate_version(&toml::Value::String("1.a.0".to_string())).is_err());
    }
}

//! License compliance validation

use super::PolicyValidator;
use crate::quality_assurance::types::{PolicyRule, QaCheck, QaCheckType, QaSeverity};
use crate::BuildContext;
use sps2_errors::Error;
use std::collections::HashSet;
use std::path::Path;
use tokio::fs;

/// License compliance validator
pub struct LicenseValidator;

impl LicenseValidator {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for LicenseValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PolicyValidator for LicenseValidator {
    fn id(&self) -> &'static str {
        "license"
    }

    fn name(&self) -> &'static str {
        "License Compliance"
    }

    async fn validate(
        &self,
        _context: &BuildContext,
        path: &Path,
        rule: &PolicyRule,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut checks = Vec::new();

        let allowed_licenses = self.get_allowed_licenses(rule);
        let forbidden_licenses = self.get_forbidden_licenses(rule);

        // Check for license files
        let license_files = find_license_files(path).await?;

        if license_files.is_empty() {
            checks.push(self.create_no_license_check(rule));
        } else {
            checks.extend(
                self.check_license_files(
                    &license_files,
                    &allowed_licenses,
                    &forbidden_licenses,
                    rule,
                )
                .await?,
            );
        }

        // Check package metadata files for license info
        checks.extend(
            check_package_metadata_licenses(
                path,
                &allowed_licenses,
                &forbidden_licenses,
                rule.severity,
            )
            .await?,
        );

        Ok(checks)
    }
}

impl LicenseValidator {
    /// Get allowed licenses from configuration or defaults
    fn get_allowed_licenses(&self, rule: &PolicyRule) -> HashSet<String> {
        rule.config
            .get("allowed_licenses")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_lowercase)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_else(|| {
                // Default allowed licenses
                [
                    "mit",
                    "apache-2.0",
                    "bsd-3-clause",
                    "bsd-2-clause",
                    "isc",
                    "cc0-1.0",
                ]
                .iter()
                .map(|&s| s.to_string())
                .collect()
            })
    }

    /// Get forbidden licenses from configuration or defaults
    fn get_forbidden_licenses(&self, rule: &PolicyRule) -> HashSet<String> {
        rule.config
            .get("forbidden_licenses")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_lowercase)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_else(|| {
                // Default forbidden licenses
                ["gpl-3.0", "agpl-3.0", "proprietary", "commercial"]
                    .iter()
                    .map(|&s| s.to_string())
                    .collect()
            })
    }

    /// Create check for missing license file
    fn create_no_license_check(&self, rule: &PolicyRule) -> QaCheck {
        QaCheck::new(
            QaCheckType::LicenseCheck,
            "license-compliance",
            rule.severity,
            "No license file found in package",
        )
        .with_context("Add a LICENSE, LICENSE.md, or COPYING file to your package")
    }

    /// Check license files for compliance
    async fn check_license_files(
        &self,
        license_files: &[std::path::PathBuf],
        allowed_licenses: &HashSet<String>,
        forbidden_licenses: &HashSet<String>,
        rule: &PolicyRule,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut checks = Vec::new();

        for license_file in license_files {
            if let Ok(content) = fs::read_to_string(license_file).await {
                checks.extend(self.check_file_content(
                    &content,
                    license_file,
                    allowed_licenses,
                    forbidden_licenses,
                    rule,
                ));
            }
        }

        Ok(checks)
    }

    /// Check individual license file content
    fn check_file_content(
        &self,
        content: &str,
        license_file: &Path,
        allowed_licenses: &HashSet<String>,
        forbidden_licenses: &HashSet<String>,
        rule: &PolicyRule,
    ) -> Vec<QaCheck> {
        let mut checks = Vec::new();
        let content_lower = content.to_lowercase();

        // Check for forbidden licenses
        for forbidden in forbidden_licenses {
            if content_lower.contains(forbidden) {
                checks.push(
                    QaCheck::new(
                        QaCheckType::LicenseCheck,
                        "license-compliance",
                        rule.severity,
                        format!("Forbidden license detected: {}", forbidden),
                    )
                    .with_location(license_file.to_path_buf(), None, None)
                    .with_context("This license is not allowed by policy"),
                );
            }
        }

        // Try to detect the license type
        if let Some(detected_license) = detect_license(&content_lower) {
            if !allowed_licenses.contains(&detected_license)
                && !forbidden_licenses.contains(&detected_license)
            {
                checks.push(
                    QaCheck::new(
                        QaCheckType::LicenseCheck,
                        "license-compliance",
                        QaSeverity::Warning,
                        format!("License '{}' is not in the allowed list", detected_license),
                    )
                    .with_location(license_file.to_path_buf(), None, None)
                    .with_context(format!("Allowed licenses: {:?}", allowed_licenses)),
                );
            }
        }

        checks
    }
}

/// Find license files in a directory
async fn find_license_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, Error> {
    let mut license_files = Vec::new();

    // Common license file names
    let license_names = [
        "LICENSE",
        "LICENSE.txt",
        "LICENSE.md",
        "License",
        "LICENCE",
        "LICENCE.txt",
        "LICENCE.md",
        "Licence",
        "COPYING",
        "COPYING.txt",
        "COPYING.md",
        "NOTICE",
        "NOTICE.txt",
        "NOTICE.md",
    ];

    for name in &license_names {
        let path = dir.join(name);
        if path.exists() {
            license_files.push(path);
        }
    }

    Ok(license_files)
}

/// Simple license detection based on content
fn detect_license(content: &str) -> Option<String> {
    // Common license identifiers
    let licenses = [
        ("mit license", "mit"),
        ("apache license, version 2.0", "apache-2.0"),
        ("apache license version 2.0", "apache-2.0"),
        ("bsd 3-clause", "bsd-3-clause"),
        ("bsd 2-clause", "bsd-2-clause"),
        ("gnu general public license v3", "gpl-3.0"),
        ("gnu general public license version 3", "gpl-3.0"),
        ("gnu affero general public license", "agpl-3.0"),
        ("isc license", "isc"),
        ("creative commons zero", "cc0-1.0"),
        ("mozilla public license 2.0", "mpl-2.0"),
        ("the unlicense", "unlicense"),
    ];

    for (pattern, license) in &licenses {
        if content.contains(pattern) {
            return Some((*license).to_string());
        }
    }

    None
}

/// Check package metadata files for license information
async fn check_package_metadata_licenses(
    dir: &Path,
    allowed_licenses: &HashSet<String>,
    forbidden_licenses: &HashSet<String>,
    severity: QaSeverity,
) -> Result<Vec<QaCheck>, Error> {
    let mut checks = Vec::new();

    // Check different package manifest files
    checks.extend(check_cargo_toml(dir, allowed_licenses, forbidden_licenses, severity).await?);
    checks.extend(check_package_json(dir, allowed_licenses, forbidden_licenses, severity).await?);
    checks.extend(check_pyproject_toml(dir, allowed_licenses, forbidden_licenses, severity).await?);

    Ok(checks)
}

/// Check Cargo.toml for license information
async fn check_cargo_toml(
    dir: &Path,
    allowed_licenses: &HashSet<String>,
    forbidden_licenses: &HashSet<String>,
    severity: QaSeverity,
) -> Result<Vec<QaCheck>, Error> {
    let cargo_toml = dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&cargo_toml).await?;
    let manifest: toml::Value = content
        .parse()
        .map_err(|e| sps2_errors::BuildError::Failed {
            message: format!("Failed to parse Cargo.toml: {}", e),
        })?;

    let Some(package) = manifest.get("package") else {
        return Ok(Vec::new());
    };

    let Some(license) = package.get("license").and_then(|v| v.as_str()) else {
        return Ok(Vec::new());
    };

    Ok(check_license_compliance(
        license,
        &cargo_toml,
        "Cargo.toml",
        allowed_licenses,
        forbidden_licenses,
        severity,
    ))
}

/// Check package.json for license information
async fn check_package_json(
    dir: &Path,
    allowed_licenses: &HashSet<String>,
    forbidden_licenses: &HashSet<String>,
    severity: QaSeverity,
) -> Result<Vec<QaCheck>, Error> {
    let package_json = dir.join("package.json");
    if !package_json.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&package_json).await?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| sps2_errors::BuildError::Failed {
            message: format!("Failed to parse package.json: {}", e),
        })?;

    let Some(license) = json.get("license").and_then(|v| v.as_str()) else {
        return Ok(Vec::new());
    };

    Ok(check_license_compliance(
        license,
        &package_json,
        "package.json",
        allowed_licenses,
        forbidden_licenses,
        severity,
    ))
}

/// Check pyproject.toml for license information
async fn check_pyproject_toml(
    dir: &Path,
    allowed_licenses: &HashSet<String>,
    forbidden_licenses: &HashSet<String>,
    severity: QaSeverity,
) -> Result<Vec<QaCheck>, Error> {
    let pyproject = dir.join("pyproject.toml");
    if !pyproject.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&pyproject).await?;
    let toml_value: toml::Value = content
        .parse()
        .map_err(|e| sps2_errors::BuildError::Failed {
            message: format!("Failed to parse pyproject.toml: {}", e),
        })?;

    let Some(project) = toml_value.get("project") else {
        return Ok(Vec::new());
    };

    let Some(license) = project.get("license").and_then(|v| v.as_str()) else {
        return Ok(Vec::new());
    };

    Ok(check_license_compliance(
        license,
        &pyproject,
        "pyproject.toml",
        allowed_licenses,
        forbidden_licenses,
        severity,
    ))
}

/// Check if a license is compliant with policy
fn check_license_compliance(
    license: &str,
    file_path: &Path,
    file_name: &str,
    allowed_licenses: &HashSet<String>,
    forbidden_licenses: &HashSet<String>,
    severity: QaSeverity,
) -> Vec<QaCheck> {
    let mut checks = Vec::new();
    let license_lower = license.to_lowercase();

    if forbidden_licenses.contains(&license_lower) {
        checks.push(
            QaCheck::new(
                QaCheckType::LicenseCheck,
                "license-compliance",
                severity,
                format!("Forbidden license in {}: {}", file_name, license),
            )
            .with_location(file_path.to_path_buf(), None, None),
        );
    } else if !allowed_licenses.contains(&license_lower) {
        checks.push(
            QaCheck::new(
                QaCheckType::LicenseCheck,
                "license-compliance",
                QaSeverity::Warning,
                format!(
                    "License '{}' in {} is not in allowed list",
                    license, file_name
                ),
            )
            .with_location(file_path.to_path_buf(), None, None),
        );
    }

    checks
}

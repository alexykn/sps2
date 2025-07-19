//! SBOM parsing for vulnerability analysis

use crate::types::{Component, ComponentIdentifier};
use serde_json::Value;
use sps2_errors::{AuditError, Error};

/// SBOM parser for extracting component information
pub struct SbomParser;

impl SbomParser {
    /// Create new SBOM parser
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Parse SBOM data to extract components
    ///
    /// # Errors
    ///
    /// Returns an error if the SBOM data cannot be parsed as valid JSON or
    /// if the SBOM format is not recognized (must be SPDX or `CycloneDX`).
    pub fn parse_sbom(&self, sbom_data: &[u8]) -> Result<Vec<Component>, Error> {
        // Try to parse as JSON
        let json: Value =
            serde_json::from_slice(sbom_data).map_err(|e| AuditError::SbomParseError {
                message: format!("Failed to parse SBOM JSON: {e}"),
            })?;

        // Detect SBOM format and parse accordingly
        if Self::is_spdx_format(&json) {
            Ok(Self::parse_spdx(&json))
        } else if Self::is_cyclonedx_format(&json) {
            Ok(Self::parse_cyclonedx(&json))
        } else {
            Err(AuditError::SbomParseError {
                message: "Unknown SBOM format".to_string(),
            }
            .into())
        }
    }

    /// Check if JSON is SPDX format
    fn is_spdx_format(json: &Value) -> bool {
        json.get("spdxVersion").is_some()
            || json.get("SPDXID").is_some()
            || json.get("packages").is_some()
    }

    /// Check if JSON is `CycloneDX` format
    fn is_cyclonedx_format(json: &Value) -> bool {
        json.get("bomFormat")
            .is_some_and(|v| v.as_str() == Some("CycloneDX"))
            || json.get("specVersion").is_some()
            || json.get("components").is_some()
    }

    /// Parse SPDX format SBOM
    fn parse_spdx(json: &Value) -> Vec<Component> {
        let mut components = Vec::new();

        if let Some(packages) = json.get("packages").and_then(|p| p.as_array()) {
            for package in packages {
                if let Some(component) = Self::parse_spdx_package(package) {
                    components.push(component);
                }
            }
        }

        components
    }

    /// Parse SPDX package
    fn parse_spdx_package(package: &Value) -> Option<Component> {
        // Skip if this is the root document package
        if package.get("SPDXID").and_then(|id| id.as_str()) == Some("SPDXRef-DOCUMENT") {
            return None;
        }

        let name = package
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string();

        let version = package
            .get("versionInfo")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Extract download location
        let download_location = package
            .get("downloadLocation")
            .and_then(|d| d.as_str())
            .map(ToString::to_string);

        // Extract license
        let license = package
            .get("licenseConcluded")
            .and_then(|l| l.as_str())
            .or_else(|| package.get("licenseDeclared").and_then(|l| l.as_str()))
            .map(ToString::to_string);

        // Extract external references for PURL/CPE
        let mut purl = None;
        let mut cpe = None;

        if let Some(external_refs) = package.get("externalRefs").and_then(|refs| refs.as_array()) {
            for ext_ref in external_refs {
                if let Some(ref_type) = ext_ref.get("referenceType").and_then(|t| t.as_str()) {
                    if let Some(locator) = ext_ref.get("referenceLocator").and_then(|l| l.as_str())
                    {
                        match ref_type {
                            "purl" => purl = Some(locator.to_string()),
                            "cpe23Type" | "cpe22Type" => cpe = Some(locator.to_string()),
                            _ => {}
                        }
                    }
                }
            }
        }

        let identifier = ComponentIdentifier {
            purl,
            cpe,
            name: name.clone(),
            version,
            package_type: "unknown".to_string(), // SPDX doesn't always specify type
        };

        Some(Component {
            identifier,
            dependencies: Vec::new(), // Would need to parse relationships
            license,
            download_location,
        })
    }

    /// Parse `CycloneDX` format SBOM
    fn parse_cyclonedx(json: &Value) -> Vec<Component> {
        let mut components = Vec::new();

        if let Some(component_list) = json.get("components").and_then(|c| c.as_array()) {
            for component in component_list {
                components.push(Self::parse_cyclonedx_component(component));
            }
        }

        components
    }

    /// Parse `CycloneDX` component
    fn parse_cyclonedx_component(component: &Value) -> Component {
        let name = component
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string();

        let version = component
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let package_type = component
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Extract PURL
        let purl = component
            .get("purl")
            .and_then(|p| p.as_str())
            .map(ToString::to_string);

        // Extract CPE (from external references)
        let mut cpe = None;
        if let Some(external_refs) = component
            .get("externalReferences")
            .and_then(|refs| refs.as_array())
        {
            for ext_ref in external_refs {
                if ext_ref.get("type").and_then(|t| t.as_str()) == Some("cpe") {
                    if let Some(url) = ext_ref.get("url").and_then(|u| u.as_str()) {
                        cpe = Some(url.to_string());
                    }
                }
            }
        }

        // Extract license
        let license = if let Some(licenses) = component.get("licenses").and_then(|l| l.as_array()) {
            licenses
                .first()
                .and_then(|license| {
                    license
                        .get("license")
                        .and_then(|l| l.get("id"))
                        .and_then(|id| id.as_str())
                        .or_else(|| {
                            license
                                .get("license")
                                .and_then(|l| l.get("name"))
                                .and_then(|name| name.as_str())
                        })
                })
                .map(ToString::to_string)
        } else {
            None
        };

        let identifier = ComponentIdentifier {
            purl,
            cpe,
            name: name.clone(),
            version,
            package_type,
        };

        Component {
            identifier,
            dependencies: Vec::new(), // Would need to parse dependencies section
            license,
            download_location: None,
        }
    }
}

impl Default for SbomParser {
    fn default() -> Self {
        Self::new()
    }
}

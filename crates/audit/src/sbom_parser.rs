//! SBOM parsing for vulnerability analysis

use crate::types::{Component, ComponentIdentifier};
use serde_json::Value;
use spsv2_errors::{AuditError, Error};

/// SBOM parser for extracting component information
pub struct SbomParser {
    /// Parser configuration
    config: ParserConfig,
}

/// Parser configuration
#[derive(Debug, Clone)]
struct ParserConfig {
    /// Maximum depth for recursive parsing
    max_depth: usize,
    /// Include dev dependencies
    include_dev_deps: bool,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            include_dev_deps: false,
        }
    }
}

impl SbomParser {
    /// Create new SBOM parser
    pub fn new() -> Self {
        Self {
            config: ParserConfig::default(),
        }
    }

    /// Parse SBOM data to extract components
    pub fn parse_sbom(&self, sbom_data: &[u8]) -> Result<Vec<Component>, Error> {
        // Try to parse as JSON
        let json: Value =
            serde_json::from_slice(sbom_data).map_err(|e| AuditError::SbomParseError {
                message: format!("Failed to parse SBOM JSON: {e}"),
            })?;

        // Detect SBOM format and parse accordingly
        if self.is_spdx_format(&json) {
            self.parse_spdx(&json)
        } else if self.is_cyclonedx_format(&json) {
            self.parse_cyclonedx(&json)
        } else {
            Err(AuditError::SbomParseError {
                message: "Unknown SBOM format".to_string(),
            }
            .into())
        }
    }

    /// Check if JSON is SPDX format
    fn is_spdx_format(&self, json: &Value) -> bool {
        json.get("spdxVersion").is_some()
            || json.get("SPDXID").is_some()
            || json.get("packages").is_some()
    }

    /// Check if JSON is CycloneDX format
    fn is_cyclonedx_format(&self, json: &Value) -> bool {
        json.get("bomFormat")
            .map_or(false, |v| v.as_str() == Some("CycloneDX"))
            || json.get("specVersion").is_some()
            || json.get("components").is_some()
    }

    /// Parse SPDX format SBOM
    fn parse_spdx(&self, json: &Value) -> Result<Vec<Component>, Error> {
        let mut components = Vec::new();

        if let Some(packages) = json.get("packages").and_then(|p| p.as_array()) {
            for package in packages {
                if let Some(component) = self.parse_spdx_package(package)? {
                    components.push(component);
                }
            }
        }

        Ok(components)
    }

    /// Parse SPDX package
    fn parse_spdx_package(&self, package: &Value) -> Result<Option<Component>, Error> {
        // Skip if this is the root document package
        if package.get("SPDXID").and_then(|id| id.as_str()) == Some("SPDXRef-DOCUMENT") {
            return Ok(None);
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

        Ok(Some(Component {
            identifier,
            dependencies: Vec::new(), // Would need to parse relationships
            license,
            download_location,
        }))
    }

    /// Parse CycloneDX format SBOM
    fn parse_cyclonedx(&self, json: &Value) -> Result<Vec<Component>, Error> {
        let mut components = Vec::new();

        if let Some(component_list) = json.get("components").and_then(|c| c.as_array()) {
            for component in component_list {
                if let Some(comp) = self.parse_cyclonedx_component(component)? {
                    components.push(comp);
                }
            }
        }

        Ok(components)
    }

    /// Parse CycloneDX component
    fn parse_cyclonedx_component(&self, component: &Value) -> Result<Option<Component>, Error> {
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

        Ok(Some(Component {
            identifier,
            dependencies: Vec::new(), // Would need to parse dependencies section
            license,
            download_location: None,
        }))
    }
}

impl Default for SbomParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SPDX: &str = r#"
    {
        "spdxVersion": "SPDX-2.3",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "test-package",
        "packages": [
            {
                "SPDXID": "SPDXRef-Package-lodash",
                "name": "lodash",
                "versionInfo": "4.17.21",
                "downloadLocation": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
                "licenseConcluded": "MIT",
                "externalRefs": [
                    {
                        "referenceCategory": "PACKAGE-MANAGER",
                        "referenceType": "purl",
                        "referenceLocator": "pkg:npm/lodash@4.17.21"
                    }
                ]
            }
        ]
    }
    "#;

    const SAMPLE_CYCLONEDX: &str = r#"
    {
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "components": [
            {
                "type": "library",
                "name": "express",
                "version": "4.18.2",
                "purl": "pkg:npm/express@4.18.2",
                "licenses": [
                    {
                        "license": {
                            "id": "MIT"
                        }
                    }
                ]
            }
        ]
    }
    "#;

    #[test]
    fn test_sbom_parser_creation() {
        let parser = SbomParser::new();
        assert_eq!(parser.config.max_depth, 10);
        assert!(!parser.config.include_dev_deps);
    }

    #[test]
    fn test_format_detection() {
        let parser = SbomParser::new();

        let spdx_json: Value = serde_json::from_str(SAMPLE_SPDX).unwrap();
        assert!(parser.is_spdx_format(&spdx_json));
        assert!(!parser.is_cyclonedx_format(&spdx_json));

        let cyclonedx_json: Value = serde_json::from_str(SAMPLE_CYCLONEDX).unwrap();
        assert!(!parser.is_spdx_format(&cyclonedx_json));
        assert!(parser.is_cyclonedx_format(&cyclonedx_json));
    }

    #[test]
    fn test_spdx_parsing() {
        let parser = SbomParser::new();
        let components = parser.parse_sbom(SAMPLE_SPDX.as_bytes()).unwrap();

        assert_eq!(components.len(), 1);

        let component = &components[0];
        assert_eq!(component.identifier.name, "lodash");
        assert_eq!(component.identifier.version, "4.17.21");
        assert_eq!(
            component.identifier.purl.as_deref(),
            Some("pkg:npm/lodash@4.17.21")
        );
        assert_eq!(component.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn test_cyclonedx_parsing() {
        let parser = SbomParser::new();
        let components = parser.parse_sbom(SAMPLE_CYCLONEDX.as_bytes()).unwrap();

        assert_eq!(components.len(), 1);

        let component = &components[0];
        assert_eq!(component.identifier.name, "express");
        assert_eq!(component.identifier.version, "4.18.2");
        assert_eq!(component.identifier.package_type, "library");
        assert_eq!(
            component.identifier.purl.as_deref(),
            Some("pkg:npm/express@4.18.2")
        );
        assert_eq!(component.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn test_invalid_sbom() {
        let parser = SbomParser::new();
        let result = parser.parse_sbom(b"invalid json");
        assert!(result.is_err());

        let result = parser.parse_sbom(b"{}");
        assert!(result.is_err());
    }
}

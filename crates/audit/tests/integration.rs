//! Integration tests for the audit crate

use spsv2_audit::{
    AuditScanner, AuditSystem, SbomParser, ScanOptions, Severity, VulnDbManager,
    VulnerabilityDatabase,
};
use tempfile::TempDir;

#[tokio::test]
async fn test_sbom_parser_integration() -> Result<(), Box<dyn std::error::Error>> {
    let parser = SbomParser::new();

    // Test with sample SPDX SBOM
    let spdx_data = r#"
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

    let components = parser.parse_sbom(spdx_data.as_bytes())?;
    assert_eq!(components.len(), 1);

    let component = &components[0];
    assert_eq!(component.identifier.name, "lodash");
    assert_eq!(component.identifier.version, "4.17.21");
    assert_eq!(
        component.identifier.purl.as_deref(),
        Some("pkg:npm/lodash@4.17.21")
    );
    assert_eq!(component.license.as_deref(), Some("MIT"));

    Ok(())
}

#[tokio::test]
async fn test_vulnerability_database_initialization() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut manager = VulnDbManager::new(&db_path)?;

    // Test initialization
    manager.initialize().await?;

    // Database file should exist
    assert!(db_path.exists());

    // Test database freshness (should be false for empty db)
    let fresh = manager.is_fresh().await?;
    assert!(!fresh);

    Ok(())
}

#[tokio::test]
async fn test_audit_scanner_basic_functionality() -> Result<(), Box<dyn std::error::Error>> {
    use spsv2_audit::{Component, ComponentIdentifier};

    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    // Create a mock vulnerability database (empty for now)
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    let vulndb = VulnerabilityDatabase::new(pool);

    let scanner = AuditScanner::new();
    let options = ScanOptions::default();

    // Create test components
    let components = vec![Component {
        identifier: ComponentIdentifier {
            purl: Some("pkg:npm/lodash@4.17.19".to_string()),
            cpe: None,
            name: "lodash".to_string(),
            version: "4.17.19".to_string(),
            package_type: "npm".to_string(),
        },
        dependencies: vec![],
        license: Some("MIT".to_string()),
        download_location: None,
    }];

    // Scan components (should return empty results since database is empty)
    let result = scanner
        .scan_components(&components, &vulndb, &options)
        .await?;

    assert_eq!(result.components_scanned, 1);
    assert!(result.vulnerabilities.is_empty());
    assert!(!result.has_critical());

    Ok(())
}

#[tokio::test]
async fn test_scan_options_configuration() {
    let default_options = ScanOptions::default();
    assert_eq!(default_options.severity_threshold, Severity::Low);
    assert!(!default_options.fail_on_critical);
    assert!(default_options.include_low_confidence);
    assert_eq!(default_options.confidence_threshold, 0.5);

    let custom_options = ScanOptions::new()
        .with_severity_threshold(Severity::High)
        .with_fail_on_critical(true)
        .with_confidence_threshold(0.8)
        .with_include_low_confidence(false);

    assert_eq!(custom_options.severity_threshold, Severity::High);
    assert!(custom_options.fail_on_critical);
    assert!(!custom_options.include_low_confidence);
    assert_eq!(custom_options.confidence_threshold, 0.8);
}

#[tokio::test]
async fn test_audit_system_creation() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let vulndb_path = temp_dir.path().join("audit_vulndb.sqlite");

    let _audit_system = AuditSystem::new(&vulndb_path)?;

    // Should create successfully (placeholder implementation)
    // In the future, this would test actual functionality

    Ok(())
}

#[tokio::test]
async fn test_cyclonedx_sbom_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let parser = SbomParser::new();

    let cyclonedx_data = r#"
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

    let components = parser.parse_sbom(cyclonedx_data.as_bytes())?;
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

    Ok(())
}

#[tokio::test]
async fn test_invalid_sbom_handling() {
    let parser = SbomParser::new();

    // Test invalid JSON
    let result = parser.parse_sbom(b"invalid json");
    assert!(result.is_err());

    // Test unknown format
    let unknown_format = r#"{"unknown": "format"}"#;
    let result = parser.parse_sbom(unknown_format.as_bytes());
    assert!(result.is_err());
}

#[tokio::test]
async fn test_database_statistics() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("stats_test.sqlite");

    let mut manager = VulnDbManager::new(&db_path)?;
    manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    let vulndb = VulnerabilityDatabase::new(pool);

    let stats = vulndb.get_statistics().await?;

    // Empty database should have zero vulnerabilities
    assert_eq!(stats.vulnerability_count, 0);
    assert!(stats.last_updated.is_none());

    Ok(())
}

#[tokio::test]
async fn test_confidence_threshold_filtering() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("confidence_test.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    let vulndb = VulnerabilityDatabase::new(pool);

    let scanner = AuditScanner::new();

    // Test with high confidence threshold
    let strict_options = ScanOptions::new()
        .with_confidence_threshold(0.9)
        .with_include_low_confidence(false);

    let components = vec![];
    let result = scanner
        .scan_components(&components, &vulndb, &strict_options)
        .await?;

    // Should work with empty components
    assert_eq!(result.components_scanned, 0);

    Ok(())
}

#[tokio::test]
async fn test_severity_filtering() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("severity_test.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    let vulndb = VulnerabilityDatabase::new(pool);

    let scanner = AuditScanner::new();

    // Test with high severity threshold
    let high_severity_options = ScanOptions::new().with_severity_threshold(Severity::High);

    let components = vec![];
    let result = scanner
        .scan_components(&components, &vulndb, &high_severity_options)
        .await?;

    assert_eq!(result.components_scanned, 0);

    // Test fail on critical option
    let fail_on_critical_options = ScanOptions::new().with_fail_on_critical(true);

    let result = scanner
        .scan_components(&components, &vulndb, &fail_on_critical_options)
        .await?;

    // Should succeed with no vulnerabilities
    assert!(!result.has_critical());

    Ok(())
}

#[tokio::test]
async fn test_malformed_sbom_edge_cases() -> Result<(), Box<dyn std::error::Error>> {
    let parser = SbomParser::new();

    // Test empty SPDX packages array
    let empty_spdx = r#"
    {
        "spdxVersion": "SPDX-2.3",
        "SPDXID": "SPDXRef-DOCUMENT",
        "packages": []
    }
    "#;

    let components = parser.parse_sbom(empty_spdx.as_bytes())?;
    assert!(components.is_empty());

    // Test empty CycloneDX components array
    let empty_cyclonedx = r#"
    {
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "components": []
    }
    "#;

    let components = parser.parse_sbom(empty_cyclonedx.as_bytes())?;
    assert!(components.is_empty());

    // Test SPDX with document package (should be skipped)
    let spdx_with_document = r#"
    {
        "spdxVersion": "SPDX-2.3",
        "SPDXID": "SPDXRef-DOCUMENT",
        "packages": [
            {
                "SPDXID": "SPDXRef-DOCUMENT",
                "name": "document-package"
            },
            {
                "SPDXID": "SPDXRef-Package-real",
                "name": "real-package",
                "versionInfo": "1.0.0"
            }
        ]
    }
    "#;

    let components = parser.parse_sbom(spdx_with_document.as_bytes())?;
    assert_eq!(components.len(), 1);
    assert_eq!(components[0].identifier.name, "real-package");

    Ok(())
}

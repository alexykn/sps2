//! Integration tests for the audit crate

use sps2_audit::{
    AuditScanner, AuditSystem, Component, ComponentIdentifier, SbomParser, ScanOptions, Severity,
    VulnDbManager, VulnerabilityDatabase,
};
use sqlx::Row;
use tempfile::TempDir;

// Test helper functions

/// Insert mock vulnerabilities into test database
async fn insert_mock_vulnerabilities(
    pool: &sqlx::SqlitePool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Insert a critical vulnerability for lodash
    sqlx::query(
        "INSERT INTO vulnerabilities (cve_id, summary, severity, cvss_score, published, modified)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("CVE-2021-23337")
    .bind("Command injection vulnerability in lodash")
    .bind("critical")
    .bind(9.8_f32)
    .bind("2021-02-15T00:00:00Z")
    .bind("2021-02-16T00:00:00Z")
    .execute(pool)
    .await?;

    let vuln_id = sqlx::query("SELECT id FROM vulnerabilities WHERE cve_id = 'CVE-2021-23337'")
        .fetch_one(pool)
        .await?
        .get::<i64, _>("id");

    // Add affected package info
    sqlx::query(
        "INSERT INTO affected_packages (vulnerability_id, package_name, package_type, affected_version, fixed_version, purl)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(vuln_id)
    .bind("lodash")
    .bind("npm")
    .bind("4.17.19")  // Exact version for simpler matching
    .bind("4.17.21")
    .bind("pkg:npm/lodash@4.17.19")
    .execute(pool)
    .await?;

    // Insert a high severity vulnerability for express
    sqlx::query(
        "INSERT INTO vulnerabilities (cve_id, summary, severity, cvss_score, published, modified)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("CVE-2022-24999")
    .bind("Path traversal vulnerability in express")
    .bind("high")
    .bind(7.5_f32)
    .bind("2022-11-26T00:00:00Z")
    .bind("2022-11-27T00:00:00Z")
    .execute(pool)
    .await?;

    let vuln_id = sqlx::query("SELECT id FROM vulnerabilities WHERE cve_id = 'CVE-2022-24999'")
        .fetch_one(pool)
        .await?
        .get::<i64, _>("id");

    sqlx::query(
        "INSERT INTO affected_packages (vulnerability_id, package_name, package_type, affected_version, fixed_version, purl, cpe)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(vuln_id)
    .bind("express")
    .bind("npm")
    .bind("4.18.1")  // Exact version for simpler matching
    .bind("4.18.2")
    .bind("pkg:npm/express@4.18.1")
    .bind("cpe:2.3:a:expressjs:express:4.18.1:*:*:*:*:*:*:*")
    .execute(pool)
    .await?;

    // Insert a medium severity vulnerability with multiple affected versions
    sqlx::query(
        "INSERT INTO vulnerabilities (cve_id, summary, severity, cvss_score, published, modified)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("CVE-2023-12345")
    .bind("Memory leak in test-lib")
    .bind("medium")
    .bind(5.3_f32)
    .bind("2023-01-15T00:00:00Z")
    .bind("2023-01-16T00:00:00Z")
    .execute(pool)
    .await?;

    let vuln_id = sqlx::query("SELECT id FROM vulnerabilities WHERE cve_id = 'CVE-2023-12345'")
        .fetch_one(pool)
        .await?
        .get::<i64, _>("id");

    // Add multiple affected versions
    for version in &["1.0.0", "1.0.1", "1.0.2"] {
        sqlx::query(
            "INSERT INTO affected_packages (vulnerability_id, package_name, package_type, affected_version, fixed_version)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(vuln_id)
        .bind("test-lib")
        .bind("generic")
        .bind(version)
        .bind("1.0.3")
        .execute(pool)
        .await?;
    }

    // Insert a low severity vulnerability
    sqlx::query(
        "INSERT INTO vulnerabilities (cve_id, summary, severity, cvss_score, published, modified)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("CVE-2023-99999")
    .bind("Information disclosure in debug-package")
    .bind("low")
    .bind(3.1_f32)
    .bind("2023-06-01T00:00:00Z")
    .bind("2023-06-02T00:00:00Z")
    .execute(pool)
    .await?;

    let vuln_id = sqlx::query("SELECT id FROM vulnerabilities WHERE cve_id = 'CVE-2023-99999'")
        .fetch_one(pool)
        .await?
        .get::<i64, _>("id");

    sqlx::query(
        "INSERT INTO affected_packages (vulnerability_id, package_name, package_type, affected_version)
         VALUES (?, ?, ?, ?)",
    )
    .bind(vuln_id)
    .bind("debug-package")
    .bind("generic")
    .bind("*") // All versions affected
    .execute(pool)
    .await?;

    // Add some references for the vulnerabilities
    let vuln_ids = vec![
        (
            "CVE-2021-23337",
            "https://nvd.nist.gov/vuln/detail/CVE-2021-23337",
        ),
        (
            "CVE-2022-24999",
            "https://nvd.nist.gov/vuln/detail/CVE-2022-24999",
        ),
        (
            "CVE-2023-12345",
            "https://example.com/advisory/CVE-2023-12345",
        ),
    ];

    for (cve_id, url) in vuln_ids {
        let vuln_id = sqlx::query("SELECT id FROM vulnerabilities WHERE cve_id = ?")
            .bind(cve_id)
            .fetch_one(pool)
            .await?
            .get::<i64, _>("id");

        sqlx::query(
            "INSERT INTO vulnerability_references (vulnerability_id, url, reference_type)
             VALUES (?, ?, ?)",
        )
        .bind(vuln_id)
        .bind(url)
        .bind("advisory")
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Create test components with various identifiers
fn create_test_components() -> Vec<Component> {
    vec![
        // Component with PURL match (vulnerable lodash)
        Component {
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
        },
        // Component with CPE match (vulnerable express)
        Component {
            identifier: ComponentIdentifier {
                purl: Some("pkg:npm/express@4.18.1".to_string()),
                cpe: Some("cpe:2.3:a:expressjs:express:4.18.1:*:*:*:*:*:*:*".to_string()),
                name: "express".to_string(),
                version: "4.18.1".to_string(),
                package_type: "npm".to_string(),
            },
            dependencies: vec![],
            license: Some("MIT".to_string()),
            download_location: None,
        },
        // Component with name match only (vulnerable test-lib)
        Component {
            identifier: ComponentIdentifier {
                purl: None,
                cpe: None,
                name: "test-lib".to_string(),
                version: "1.0.1".to_string(),
                package_type: "generic".to_string(),
            },
            dependencies: vec![],
            license: None,
            download_location: None,
        },
        // Safe component (fixed lodash version)
        Component {
            identifier: ComponentIdentifier {
                purl: Some("pkg:npm/lodash@4.17.21".to_string()),
                cpe: None,
                name: "lodash".to_string(),
                version: "4.17.21".to_string(),
                package_type: "npm".to_string(),
            },
            dependencies: vec![],
            license: Some("MIT".to_string()),
            download_location: None,
        },
        // Safe component (no vulnerabilities)
        Component {
            identifier: ComponentIdentifier {
                purl: Some("pkg:npm/safe-package@1.0.0".to_string()),
                cpe: None,
                name: "safe-package".to_string(),
                version: "1.0.0".to_string(),
                package_type: "npm".to_string(),
            },
            dependencies: vec![],
            license: Some("Apache-2.0".to_string()),
            download_location: None,
        },
    ]
}

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
async fn test_full_audit_workflow() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;

    // Insert mock vulnerabilities
    insert_mock_vulnerabilities(&pool).await?;

    let vulndb = VulnerabilityDatabase::new(pool);
    let scanner = AuditScanner::new();
    let options = ScanOptions::default();

    // Create test components
    let components = create_test_components();

    // Scan components
    let result = scanner
        .scan_components(&components, &vulndb, &options)
        .await?;

    // Verify results
    assert_eq!(result.components_scanned, 5);
    assert!(!result.vulnerabilities.is_empty());

    // Should find vulnerabilities for lodash, express, and test-lib
    // Note: The scanner may find multiple matches per component (name, PURL, CPE)
    assert!(result.vulnerabilities.len() >= 3);

    // Check for critical vulnerability (lodash)
    assert!(result.has_critical());

    // Verify severity counts
    assert!(result.count_by_severity(Severity::Critical) >= 1);
    assert!(result.count_by_severity(Severity::High) >= 1);
    assert!(result.count_by_severity(Severity::Medium) >= 1);

    // Verify match reasons
    let lodash_match = result
        .vulnerabilities
        .iter()
        .find(|v| v.vulnerability.cve_id == "CVE-2021-23337");
    assert!(lodash_match.is_some());

    let express_match = result
        .vulnerabilities
        .iter()
        .find(|v| v.vulnerability.cve_id == "CVE-2022-24999");
    assert!(express_match.is_some());

    Ok(())
}

#[tokio::test]
async fn test_vulnerability_matching() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    insert_mock_vulnerabilities(&pool).await?;

    let vulndb = VulnerabilityDatabase::new(pool);
    let scanner = AuditScanner::new();
    let options = ScanOptions::default();

    // Test PURL matching
    let purl_component = vec![Component {
        identifier: ComponentIdentifier {
            purl: Some("pkg:npm/lodash@4.17.19".to_string()),
            cpe: None,
            name: "lodash".to_string(),
            version: "4.17.19".to_string(),
            package_type: "npm".to_string(),
        },
        dependencies: vec![],
        license: None,
        download_location: None,
    }];

    let result = scanner
        .scan_components(&purl_component, &vulndb, &options)
        .await?;

    assert!(!result.vulnerabilities.is_empty());
    // Should find vulnerability through either PURL or name matching
    assert!(result
        .vulnerabilities
        .iter()
        .any(|v| v.vulnerability.cve_id == "CVE-2021-23337"));

    // Test CPE matching
    let cpe_component = vec![Component {
        identifier: ComponentIdentifier {
            purl: None,
            cpe: Some("cpe:2.3:a:expressjs:express:4.18.1:*:*:*:*:*:*:*".to_string()),
            name: "express".to_string(),
            version: "4.18.1".to_string(),
            package_type: "npm".to_string(),
        },
        dependencies: vec![],
        license: None,
        download_location: None,
    }];

    let result = scanner
        .scan_components(&cpe_component, &vulndb, &options)
        .await?;

    assert!(!result.vulnerabilities.is_empty());
    // Should find vulnerability through either CPE or name matching
    assert!(result
        .vulnerabilities
        .iter()
        .any(|v| v.vulnerability.cve_id == "CVE-2022-24999"));

    // Test name/version matching
    let name_component = vec![Component {
        identifier: ComponentIdentifier {
            purl: None,
            cpe: None,
            name: "test-lib".to_string(),
            version: "1.0.1".to_string(),
            package_type: "generic".to_string(),
        },
        dependencies: vec![],
        license: None,
        download_location: None,
    }];

    let result = scanner
        .scan_components(&name_component, &vulndb, &options)
        .await?;

    assert!(!result.vulnerabilities.is_empty());
    // Should find vulnerability through name matching
    assert!(result
        .vulnerabilities
        .iter()
        .any(|v| v.vulnerability.cve_id == "CVE-2023-12345"));

    Ok(())
}

#[tokio::test]
async fn test_audit_report_generation() -> Result<(), Box<dyn std::error::Error>> {
    use sps2_audit::{AuditReport, PackageAudit};
    use sps2_types::Version;

    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    insert_mock_vulnerabilities(&pool).await?;

    let vulndb = VulnerabilityDatabase::new(pool);
    let scanner = AuditScanner::new();
    let options = ScanOptions::default();

    // Scan multiple packages
    let components1 = vec![create_test_components()[0].clone()]; // vulnerable lodash
    let components2 = vec![create_test_components()[1].clone()]; // vulnerable express
    let components3 = vec![create_test_components()[4].clone()]; // safe package

    let scan1 = scanner
        .scan_components(&components1, &vulndb, &options)
        .await?;
    let scan2 = scanner
        .scan_components(&components2, &vulndb, &options)
        .await?;
    let scan3 = scanner
        .scan_components(&components3, &vulndb, &options)
        .await?;

    // Create package audits
    let audits = vec![
        PackageAudit {
            package_name: "lodash".to_string(),
            package_version: Version::parse("4.17.19").unwrap(),
            components: 1,
            vulnerabilities: scan1.vulnerabilities,
            scan_timestamp: chrono::Utc::now(),
        },
        PackageAudit {
            package_name: "express".to_string(),
            package_version: Version::parse("4.18.1").unwrap(),
            components: 1,
            vulnerabilities: scan2.vulnerabilities,
            scan_timestamp: chrono::Utc::now(),
        },
        PackageAudit {
            package_name: "safe-package".to_string(),
            package_version: Version::parse("1.0.0").unwrap(),
            components: 1,
            vulnerabilities: scan3.vulnerabilities,
            scan_timestamp: chrono::Utc::now(),
        },
    ];

    // Generate report
    let report = AuditReport::new(audits);

    // Verify report statistics
    assert_eq!(report.summary.packages_scanned, 3);
    assert_eq!(report.summary.vulnerable_packages, 2); // lodash and express
    assert!(report.summary.total_vulnerabilities >= 2);
    assert!(report.summary.critical_count >= 1); // lodash CVE (might have multiple matches)
    assert!(report.summary.high_count >= 1); // express CVE (might have multiple matches)

    // Verify critical packages
    let critical_packages = report.critical_packages();
    assert_eq!(critical_packages.len(), 1);
    assert_eq!(critical_packages[0].package_name, "lodash");

    Ok(())
}

#[tokio::test]
async fn test_critical_vulnerability_failure() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    insert_mock_vulnerabilities(&pool).await?;

    let vulndb = VulnerabilityDatabase::new(pool);
    let scanner = AuditScanner::new();

    // Configure to fail on critical
    let options = ScanOptions::new().with_fail_on_critical(true);

    // Create component with critical vulnerability
    let components = vec![create_test_components()[0].clone()]; // vulnerable lodash

    // Scan should fail due to critical vulnerability
    let result = scanner
        .scan_components(&components, &vulndb, &options)
        .await;

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.to_string().contains("critical vulnerabilities found"));

    // Test with safe component - should succeed
    let safe_components = vec![create_test_components()[4].clone()]; // safe package
    let result = scanner
        .scan_components(&safe_components, &vulndb, &options)
        .await;

    assert!(result.is_ok());

    Ok(())
}

#[tokio::test]
async fn test_vulnerability_database_queries() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    insert_mock_vulnerabilities(&pool).await?;

    let vulndb = VulnerabilityDatabase::new(pool);

    // Test find by package name and version
    let vulns = vulndb
        .find_vulnerabilities_by_package("lodash", "4.17.19")
        .await?;
    assert!(!vulns.is_empty());
    assert!(vulns.iter().any(|v| v.cve_id == "CVE-2021-23337"));

    // Test find by PURL
    let vulns = vulndb
        .find_vulnerabilities_by_purl("pkg:npm/express@4.18.1")
        .await?;
    assert!(!vulns.is_empty());
    assert!(vulns.iter().any(|v| v.cve_id == "CVE-2022-24999"));

    // Test find by CPE
    let vulns = vulndb
        .find_vulnerabilities_by_cpe("cpe:2.3:a:expressjs:express:4.18.1:*:*:*:*:*:*:*")
        .await?;
    assert!(!vulns.is_empty());

    // Test get specific vulnerability
    let vuln = vulndb.get_vulnerability_by_cve("CVE-2021-23337").await?;
    assert!(vuln.is_some());
    let vuln = vuln.unwrap();
    assert_eq!(vuln.severity, Severity::Critical);
    assert_eq!(vuln.cvss_score, Some(9.8));
    assert!(!vuln.references.is_empty());

    // Test database statistics
    let stats = vulndb.get_statistics().await?;
    assert_eq!(stats.vulnerability_count, 4); // We inserted 4 vulnerabilities

    Ok(())
}

#[tokio::test]
async fn test_severity_filtering_advanced() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    insert_mock_vulnerabilities(&pool).await?;

    let vulndb = VulnerabilityDatabase::new(pool);
    let scanner = AuditScanner::new();

    // Test components with all severity levels
    let components = create_test_components();

    // Test filtering - only high and critical
    let high_threshold_options = ScanOptions::new().with_severity_threshold(Severity::High);

    let result = scanner
        .scan_components(&components, &vulndb, &high_threshold_options)
        .await?;

    // Should only include critical and high vulnerabilities
    for vuln_match in &result.vulnerabilities {
        assert!(vuln_match.vulnerability.severity >= Severity::High);
    }

    // Test filtering - only critical
    let critical_threshold_options = ScanOptions::new().with_severity_threshold(Severity::Critical);

    let result = scanner
        .scan_components(&components, &vulndb, &critical_threshold_options)
        .await?;

    // Should only include critical vulnerabilities
    for vuln_match in &result.vulnerabilities {
        assert_eq!(vuln_match.vulnerability.severity, Severity::Critical);
    }
    assert!(result.count_by_severity(Severity::Critical) >= 1);
    // count_by_severity counts >= severity, so we check that all are critical
    assert_eq!(
        result.vulnerabilities.len(),
        result.count_by_severity(Severity::Critical)
    );

    Ok(())
}

#[tokio::test]
async fn test_confidence_scoring() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;
    insert_mock_vulnerabilities(&pool).await?;

    let vulndb = VulnerabilityDatabase::new(pool);
    let scanner = AuditScanner::new();

    // Test with low confidence threshold
    let low_confidence_options = ScanOptions::new()
        .with_confidence_threshold(0.3)
        .with_include_low_confidence(true);

    let components = create_test_components();
    let low_conf_result = scanner
        .scan_components(&components, &vulndb, &low_confidence_options)
        .await?;

    // Test with high confidence threshold
    let high_confidence_options = ScanOptions::new()
        .with_confidence_threshold(0.9)
        .with_include_low_confidence(false);

    let high_conf_result = scanner
        .scan_components(&components, &vulndb, &high_confidence_options)
        .await?;

    // High confidence should have fewer or equal matches
    assert!(high_conf_result.vulnerabilities.len() <= low_conf_result.vulnerabilities.len());

    // All matches should meet confidence threshold
    for vuln_match in &high_conf_result.vulnerabilities {
        assert!(vuln_match.confidence >= 0.9);
    }

    Ok(())
}

#[tokio::test]
async fn test_version_matching_logic() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_vulndb.sqlite");

    let mut db_manager = VulnDbManager::new(&db_path)?;
    db_manager.initialize().await?;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;

    // Insert a vulnerability with specific version range
    sqlx::query(
        "INSERT INTO vulnerabilities (cve_id, summary, severity, published, modified)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind("CVE-2024-TEST")
    .bind("Test version matching")
    .bind("high")
    .bind("2024-01-01T00:00:00Z")
    .bind("2024-01-01T00:00:00Z")
    .execute(&pool)
    .await?;

    let vuln_id = sqlx::query("SELECT id FROM vulnerabilities WHERE cve_id = 'CVE-2024-TEST'")
        .fetch_one(&pool)
        .await?
        .get::<i64, _>("id");

    // Add affected versions
    sqlx::query(
        "INSERT INTO affected_packages (vulnerability_id, package_name, package_type, affected_version, fixed_version)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(vuln_id)
    .bind("version-test")
    .bind("generic")
    .bind(">=2.0.0")
    .bind("2.5.0")
    .execute(&pool)
    .await?;

    let vulndb = VulnerabilityDatabase::new(pool);
    let scanner = AuditScanner::new();
    let options = ScanOptions::default();

    // Test affected version (2.3.0 is >= 2.0.0 and < 2.5.0)
    let affected_component = vec![Component {
        identifier: ComponentIdentifier {
            purl: None,
            cpe: None,
            name: "version-test".to_string(),
            version: "2.3.0".to_string(),
            package_type: "generic".to_string(),
        },
        dependencies: vec![],
        license: None,
        download_location: None,
    }];

    let result = scanner
        .scan_components(&affected_component, &vulndb, &options)
        .await?;

    assert!(
        !result.vulnerabilities.is_empty(),
        "Version 2.3.0 should be affected"
    );

    // Test fixed version (2.5.0)
    let fixed_component = vec![Component {
        identifier: ComponentIdentifier {
            purl: None,
            cpe: None,
            name: "version-test".to_string(),
            version: "2.5.0".to_string(),
            package_type: "generic".to_string(),
        },
        dependencies: vec![],
        license: None,
        download_location: None,
    }];

    let result = scanner
        .scan_components(&fixed_component, &vulndb, &options)
        .await?;

    assert!(
        result.vulnerabilities.is_empty(),
        "Version 2.5.0 should be fixed"
    );

    // Test unaffected version (1.9.0 is < 2.0.0)
    let unaffected_component = vec![Component {
        identifier: ComponentIdentifier {
            purl: None,
            cpe: None,
            name: "version-test".to_string(),
            version: "1.9.0".to_string(),
            package_type: "generic".to_string(),
        },
        dependencies: vec![],
        license: None,
        download_location: None,
    }];

    let result = scanner
        .scan_components(&unaffected_component, &vulndb, &options)
        .await?;

    assert!(
        result.vulnerabilities.is_empty(),
        "Version 1.9.0 should not be affected"
    );

    Ok(())
}

#[tokio::test]
async fn test_audit_scanner_basic_functionality() -> Result<(), Box<dyn std::error::Error>> {
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
    // With the unified schema management, metadata is initialized with last_update = "0"
    assert!(stats.last_updated.is_some());

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

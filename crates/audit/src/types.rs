//! Types for CVE audit system

use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::collections::HashMap;

/// Vulnerability severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Low severity
    Low,
    /// Medium severity
    Medium,
    /// High severity
    High,
    /// Critical severity
    Critical,
}

impl Default for Severity {
    fn default() -> Self {
        Self::Low
    }
}

/// Vulnerability information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vulnerability {
    /// CVE identifier
    pub cve_id: String,
    /// Vulnerability summary
    pub summary: String,
    /// Severity level
    pub severity: Severity,
    /// CVSS score
    pub cvss_score: Option<f32>,
    /// Affected versions
    pub affected_versions: Vec<String>,
    /// Fixed versions
    pub fixed_versions: Vec<String>,
    /// Published date
    pub published: chrono::DateTime<chrono::Utc>,
    /// Last modified date
    pub modified: chrono::DateTime<chrono::Utc>,
    /// References (URLs)
    pub references: Vec<String>,
}

/// Software component identifier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentIdentifier {
    /// Package URL (PURL)
    pub purl: Option<String>,
    /// Common Platform Enumeration (CPE)
    pub cpe: Option<String>,
    /// Package name
    pub name: String,
    /// Package version
    pub version: String,
    /// Package type
    pub package_type: String,
}

/// Software component from SBOM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    /// Component identifier
    pub identifier: ComponentIdentifier,
    /// Dependencies
    pub dependencies: Vec<ComponentIdentifier>,
    /// License information
    pub license: Option<String>,
    /// Download location
    pub download_location: Option<String>,
}

/// Vulnerability match for a component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnerabilityMatch {
    /// The vulnerability
    pub vulnerability: Vulnerability,
    /// Affected component
    pub component: ComponentIdentifier,
    /// Match confidence (0.0-1.0)
    pub confidence: f32,
    /// Match reason
    pub match_reason: String,
}

/// Audit result for a single package
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageAudit {
    /// Package name
    pub package_name: String,
    /// Package version
    pub package_version: Version,
    /// Number of components scanned
    pub components: usize,
    /// Vulnerabilities found
    pub vulnerabilities: Vec<VulnerabilityMatch>,
    /// Scan timestamp
    pub scan_timestamp: chrono::DateTime<chrono::Utc>,
}

impl PackageAudit {
    /// Get vulnerabilities by severity
    pub fn vulnerabilities_by_severity(&self, severity: Severity) -> Vec<&VulnerabilityMatch> {
        self.vulnerabilities
            .iter()
            .filter(|v| v.vulnerability.severity >= severity)
            .collect()
    }

    /// Count vulnerabilities by severity
    pub fn count_by_severity(&self, severity: Severity) -> usize {
        self.vulnerabilities_by_severity(severity).len()
    }

    /// Check if package has critical vulnerabilities
    pub fn has_critical(&self) -> bool {
        self.count_by_severity(Severity::Critical) > 0
    }

    /// Check if package has high or critical vulnerabilities
    pub fn has_high_or_critical(&self) -> bool {
        self.count_by_severity(Severity::High) > 0
    }
}

/// Complete audit report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    /// Package audits
    pub package_audits: Vec<PackageAudit>,
    /// Scan timestamp
    pub scan_timestamp: chrono::DateTime<chrono::Utc>,
    /// Summary statistics
    pub summary: AuditSummary,
}

impl AuditReport {
    /// Create new audit report
    pub fn new(package_audits: Vec<PackageAudit>) -> Self {
        let summary = AuditSummary::from_audits(&package_audits);

        Self {
            package_audits,
            scan_timestamp: chrono::Utc::now(),
            summary,
        }
    }

    /// Get total number of vulnerabilities
    pub fn total_vulnerabilities(&self) -> usize {
        self.summary.total_vulnerabilities
    }

    /// Get count of critical vulnerabilities
    pub fn critical_count(&self) -> usize {
        self.summary.critical_count
    }

    /// Get packages with critical vulnerabilities
    pub fn critical_packages(&self) -> Vec<&PackageAudit> {
        self.package_audits
            .iter()
            .filter(|audit| audit.has_critical())
            .collect()
    }

    /// Check if any package has critical vulnerabilities
    pub fn has_critical_vulnerabilities(&self) -> bool {
        self.critical_count() > 0
    }
}

/// Audit summary statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSummary {
    /// Total packages scanned
    pub packages_scanned: usize,
    /// Total vulnerabilities found
    pub total_vulnerabilities: usize,
    /// Count by severity
    pub severity_counts: HashMap<String, usize>,
    /// Critical vulnerability count
    pub critical_count: usize,
    /// High vulnerability count
    pub high_count: usize,
    /// Medium vulnerability count
    pub medium_count: usize,
    /// Low vulnerability count
    pub low_count: usize,
    /// Packages with vulnerabilities
    pub vulnerable_packages: usize,
}

impl AuditSummary {
    /// Create summary from package audits
    pub fn from_audits(audits: &[PackageAudit]) -> Self {
        let packages_scanned = audits.len();
        let total_vulnerabilities = audits.iter().map(|audit| audit.vulnerabilities.len()).sum();

        let critical_count = audits
            .iter()
            .map(|audit| audit.count_by_severity(Severity::Critical))
            .sum();

        let high_count = audits
            .iter()
            .map(|audit| audit.count_by_severity(Severity::High))
            .sum();

        let medium_count = audits
            .iter()
            .map(|audit| audit.count_by_severity(Severity::Medium))
            .sum();

        let low_count = audits
            .iter()
            .map(|audit| audit.count_by_severity(Severity::Low))
            .sum();

        let vulnerable_packages = audits
            .iter()
            .filter(|audit| !audit.vulnerabilities.is_empty())
            .count();

        let mut severity_counts = HashMap::new();
        severity_counts.insert("critical".to_string(), critical_count);
        severity_counts.insert("high".to_string(), high_count);
        severity_counts.insert("medium".to_string(), medium_count);
        severity_counts.insert("low".to_string(), low_count);

        Self {
            packages_scanned,
            total_vulnerabilities,
            severity_counts,
            critical_count,
            high_count,
            medium_count,
            low_count,
            vulnerable_packages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
    }

    #[test]
    fn test_vulnerability_creation() {
        let vuln = Vulnerability {
            cve_id: "CVE-2023-1234".to_string(),
            summary: "Test vulnerability".to_string(),
            severity: Severity::High,
            cvss_score: Some(7.5),
            affected_versions: vec!["1.0.0".to_string()],
            fixed_versions: vec!["1.0.1".to_string()],
            published: chrono::Utc::now(),
            modified: chrono::Utc::now(),
            references: vec!["https://example.com/cve".to_string()],
        };

        assert_eq!(vuln.cve_id, "CVE-2023-1234");
        assert_eq!(vuln.severity, Severity::High);
        assert_eq!(vuln.cvss_score, Some(7.5));
    }

    #[test]
    fn test_package_audit() {
        let audit = PackageAudit {
            package_name: "test-package".to_string(),
            package_version: Version::parse("1.0.0").unwrap(),
            components: 5,
            vulnerabilities: vec![],
            scan_timestamp: chrono::Utc::now(),
        };

        assert_eq!(audit.package_name, "test-package");
        assert_eq!(audit.components, 5);
        assert!(!audit.has_critical());
        assert_eq!(audit.count_by_severity(Severity::Critical), 0);
    }

    #[test]
    fn test_audit_summary() {
        let audits = vec![
            PackageAudit {
                package_name: "pkg1".to_string(),
                package_version: Version::parse("1.0.0").unwrap(),
                components: 3,
                vulnerabilities: vec![],
                scan_timestamp: chrono::Utc::now(),
            },
            PackageAudit {
                package_name: "pkg2".to_string(),
                package_version: Version::parse("2.0.0").unwrap(),
                components: 7,
                vulnerabilities: vec![],
                scan_timestamp: chrono::Utc::now(),
            },
        ];

        let summary = AuditSummary::from_audits(&audits);

        assert_eq!(summary.packages_scanned, 2);
        assert_eq!(summary.total_vulnerabilities, 0);
        assert_eq!(summary.vulnerable_packages, 0);
        assert_eq!(summary.critical_count, 0);
    }

    #[test]
    fn test_audit_report() {
        let audits = vec![PackageAudit {
            package_name: "test".to_string(),
            package_version: Version::parse("1.0.0").unwrap(),
            components: 1,
            vulnerabilities: vec![],
            scan_timestamp: chrono::Utc::now(),
        }];

        let report = AuditReport::new(audits);

        assert_eq!(report.package_audits.len(), 1);
        assert_eq!(report.total_vulnerabilities(), 0);
        assert!(!report.has_critical_vulnerabilities());
    }
}

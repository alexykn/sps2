//! CVE scanning engine

use crate::{
    types::{Component, Severity, Vulnerability, VulnerabilityMatch},
    vulndb::VulnerabilityDatabase,
};
use serde::{Deserialize, Serialize};
use spsv2_errors::{AuditError, Error};

/// Options for vulnerability scanning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptions {
    /// Minimum severity to report
    pub severity_threshold: Severity,
    /// Fail on critical vulnerabilities
    pub fail_on_critical: bool,
    /// Include low confidence matches
    pub include_low_confidence: bool,
    /// Minimum confidence threshold (0.0-1.0)
    pub confidence_threshold: f32,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            severity_threshold: Severity::Low,
            fail_on_critical: false,
            include_low_confidence: true,
            confidence_threshold: 0.5,
        }
    }
}

impl ScanOptions {
    /// Create new scan options
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set severity threshold
    pub fn with_severity_threshold(mut self, threshold: Severity) -> Self {
        self.severity_threshold = threshold;
        self
    }

    /// Set fail on critical
    pub fn with_fail_on_critical(mut self, fail: bool) -> Self {
        self.fail_on_critical = fail;
        self
    }

    /// Set confidence threshold
    pub fn with_confidence_threshold(mut self, threshold: f32) -> Self {
        self.confidence_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Include low confidence matches
    pub fn with_include_low_confidence(mut self, include: bool) -> Self {
        self.include_low_confidence = include;
        self
    }
}

/// Result of vulnerability scanning
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Vulnerabilities found
    pub vulnerabilities: Vec<VulnerabilityMatch>,
    /// Components scanned
    pub components_scanned: usize,
    /// Scan duration
    pub scan_duration: std::time::Duration,
}

impl ScanResult {
    /// Check if scan found critical vulnerabilities
    pub fn has_critical(&self) -> bool {
        self.vulnerabilities
            .iter()
            .any(|v| v.vulnerability.severity == Severity::Critical)
    }

    /// Get vulnerabilities by severity
    pub fn by_severity(&self, severity: Severity) -> Vec<&VulnerabilityMatch> {
        self.vulnerabilities
            .iter()
            .filter(|v| v.vulnerability.severity >= severity)
            .collect()
    }

    /// Count vulnerabilities by severity
    pub fn count_by_severity(&self, severity: Severity) -> usize {
        self.by_severity(severity).len()
    }
}

/// CVE scanner
pub struct AuditScanner {
    /// Scanner configuration
    #[allow(dead_code)] // Configuration fields reserved for future implementation
    config: ScannerConfig,
}

/// Scanner configuration
#[derive(Debug, Clone)]
struct ScannerConfig {
    /// Maximum concurrent scans
    #[allow(dead_code)] // Will be used when parallel scanning is implemented
    max_concurrent: usize,
    /// Scan timeout per component
    #[allow(dead_code)] // Will be used for timeout handling in production scanning
    component_timeout: std::time::Duration,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 10,
            component_timeout: std::time::Duration::from_secs(30),
        }
    }
}

impl AuditScanner {
    /// Create new audit scanner
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: ScannerConfig::default(),
        }
    }

    /// Scan components for vulnerabilities
    pub async fn scan_components(
        &self,
        components: &[Component],
        vulndb: &VulnerabilityDatabase,
        options: &ScanOptions,
    ) -> Result<ScanResult, Error> {
        let start_time = std::time::Instant::now();
        let mut vulnerabilities = Vec::new();

        // For now, implement a simple sequential scan
        // In the future, this would be parallelized
        for component in components {
            let matches = self.scan_component(component, vulndb, options).await?;
            vulnerabilities.extend(matches);
        }

        // Filter by confidence threshold
        vulnerabilities.retain(|v| v.confidence >= options.confidence_threshold);

        // Filter by severity threshold
        vulnerabilities.retain(|v| v.vulnerability.severity >= options.severity_threshold);

        let scan_duration = start_time.elapsed();

        let result = ScanResult {
            vulnerabilities,
            components_scanned: components.len(),
            scan_duration,
        };

        // Check if we should fail on critical vulnerabilities
        if options.fail_on_critical && result.has_critical() {
            return Err(AuditError::CriticalVulnerabilitiesFound {
                count: result.count_by_severity(Severity::Critical),
            }
            .into());
        }

        Ok(result)
    }

    /// Scan single component for vulnerabilities
    async fn scan_component(
        &self,
        component: &Component,
        vulndb: &VulnerabilityDatabase,
        _options: &ScanOptions,
    ) -> Result<Vec<VulnerabilityMatch>, Error> {
        let mut matches = Vec::new();

        // Search by package name and version
        let vulnerabilities = vulndb
            .find_vulnerabilities_by_package(
                &component.identifier.name,
                &component.identifier.version,
            )
            .await?;

        for vulnerability in vulnerabilities {
            // Check if the component version is affected
            if self.is_version_affected(&component.identifier.version, &vulnerability) {
                let confidence = self.calculate_confidence(component, &vulnerability);

                matches.push(VulnerabilityMatch {
                    vulnerability,
                    component: component.identifier.clone(),
                    confidence,
                    match_reason: "Package name and version match".to_string(),
                });
            }
        }

        // Search by PURL if available
        if let Some(purl) = &component.identifier.purl {
            let purl_vulnerabilities = vulndb.find_vulnerabilities_by_purl(purl).await?;

            for vulnerability in purl_vulnerabilities {
                let confidence = self.calculate_confidence(component, &vulnerability);

                matches.push(VulnerabilityMatch {
                    vulnerability,
                    component: component.identifier.clone(),
                    confidence,
                    match_reason: "PURL match".to_string(),
                });
            }
        }

        // Search by CPE if available
        if let Some(cpe) = &component.identifier.cpe {
            let cpe_vulnerabilities = vulndb.find_vulnerabilities_by_cpe(cpe).await?;

            for vulnerability in cpe_vulnerabilities {
                let confidence = self.calculate_confidence(component, &vulnerability);

                matches.push(VulnerabilityMatch {
                    vulnerability,
                    component: component.identifier.clone(),
                    confidence,
                    match_reason: "CPE match".to_string(),
                });
            }
        }

        // Deduplicate matches by CVE ID
        matches.sort_by(|a, b| a.vulnerability.cve_id.cmp(&b.vulnerability.cve_id));
        matches.dedup_by(|a, b| a.vulnerability.cve_id == b.vulnerability.cve_id);

        Ok(matches)
    }

    /// Check if a version is affected by a vulnerability
    fn is_version_affected(&self, version: &str, vulnerability: &Vulnerability) -> bool {
        // Simplified version checking - in practice this would use semver
        // and handle version ranges properly

        // Check if version is in affected versions
        if vulnerability
            .affected_versions
            .contains(&version.to_string())
        {
            return true;
        }

        // Check if version is before any fixed version
        for fixed_version in &vulnerability.fixed_versions {
            if let (Ok(current), Ok(fixed)) = (
                semver::Version::parse(version),
                semver::Version::parse(fixed_version),
            ) {
                if current < fixed {
                    return true;
                }
            }
        }

        false
    }

    /// Calculate match confidence (0.0-1.0)
    fn calculate_confidence(&self, component: &Component, vulnerability: &Vulnerability) -> f32 {
        let mut confidence = 0.0_f32;

        // Base confidence for package name match
        confidence += 0.6;

        // Extra confidence for exact version match
        if vulnerability
            .affected_versions
            .contains(&component.identifier.version)
        {
            confidence += 0.3;
        }

        // Extra confidence for PURL match
        if component.identifier.purl.is_some() {
            confidence += 0.1;
        }

        confidence.min(1.0)
    }
}

impl Default for AuditScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ComponentIdentifier, Vulnerability};

    #[test]
    fn test_scan_options() {
        let options = ScanOptions::default();
        assert_eq!(options.severity_threshold, Severity::Low);
        assert!(!options.fail_on_critical);
        assert!((options.confidence_threshold - 0.5).abs() < f32::EPSILON);

        let custom_options = ScanOptions::new()
            .with_severity_threshold(Severity::High)
            .with_fail_on_critical(true)
            .with_confidence_threshold(0.8);

        assert_eq!(custom_options.severity_threshold, Severity::High);
        assert!(custom_options.fail_on_critical);
        assert!((custom_options.confidence_threshold - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_scan_result() {
        let result = ScanResult {
            vulnerabilities: vec![],
            components_scanned: 5,
            scan_duration: std::time::Duration::from_millis(100),
        };

        assert!(!result.has_critical());
        assert_eq!(result.components_scanned, 5);
        assert_eq!(result.count_by_severity(Severity::Critical), 0);
    }

    #[test]
    fn test_scanner_creation() {
        let scanner = AuditScanner::new();
        assert_eq!(scanner.config.max_concurrent, 10);
        assert_eq!(
            scanner.config.component_timeout,
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn test_version_affected() {
        let scanner = AuditScanner::new();

        let vulnerability = Vulnerability {
            cve_id: "CVE-2023-1234".to_string(),
            summary: "Test vulnerability".to_string(),
            severity: Severity::High,
            cvss_score: Some(7.5),
            affected_versions: vec!["1.0.0".to_string()],
            fixed_versions: vec!["1.0.1".to_string()],
            published: chrono::Utc::now(),
            modified: chrono::Utc::now(),
            references: vec![],
        };

        // Exact match in affected versions
        assert!(scanner.is_version_affected("1.0.0", &vulnerability));

        // Version before fix (would need proper semver parsing)
        // This is a simplified test
        assert!(!scanner.is_version_affected("1.0.1", &vulnerability));
    }

    #[test]
    fn test_confidence_calculation() {
        let scanner = AuditScanner::new();

        let component = Component {
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
        };

        let vulnerability = Vulnerability {
            cve_id: "CVE-2023-1234".to_string(),
            summary: "Test vulnerability".to_string(),
            severity: Severity::High,
            cvss_score: Some(7.5),
            affected_versions: vec!["4.17.19".to_string()],
            fixed_versions: vec!["4.17.20".to_string()],
            published: chrono::Utc::now(),
            modified: chrono::Utc::now(),
            references: vec![],
        };

        let confidence = scanner.calculate_confidence(&component, &vulnerability);

        // Should have high confidence due to exact version match and PURL
        assert!(confidence > 0.8);
    }
}

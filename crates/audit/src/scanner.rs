//! CVE scanning engine

use crate::{
    types::{Component, Severity, Vulnerability, VulnerabilityMatch},
    vulndb::VulnerabilityDatabase,
};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use sps2_errors::{AuditError, Error};
use std::sync::Arc;
use tokio::sync::Semaphore;

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
    config: ScannerConfig,
}

/// Scanner configuration
#[derive(Debug, Clone)]
pub struct ScannerConfig {
    /// Maximum concurrent scans
    pub max_concurrent: usize,
    /// Scan timeout per component
    pub component_timeout: std::time::Duration,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 10,
            component_timeout: std::time::Duration::from_secs(30),
        }
    }
}

impl ScannerConfig {
    /// Create new scanner configuration
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum concurrent scans
    pub fn with_max_concurrent(mut self, max_concurrent: usize) -> Self {
        self.max_concurrent = max_concurrent.max(1); // Ensure at least 1
        self
    }

    /// Set component scan timeout
    pub fn with_component_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.component_timeout = timeout;
        self
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

    /// Create new audit scanner with custom configuration
    #[must_use]
    pub fn with_config(config: ScannerConfig) -> Self {
        Self { config }
    }

    /// Scan components for vulnerabilities with parallel execution
    pub async fn scan_components(
        &self,
        components: &[Component],
        vulndb: &VulnerabilityDatabase,
        options: &ScanOptions,
    ) -> Result<ScanResult, Error> {
        let start_time = std::time::Instant::now();

        if components.is_empty() {
            return Ok(ScanResult {
                vulnerabilities: Vec::new(),
                components_scanned: 0,
                scan_duration: start_time.elapsed(),
            });
        }

        // Create a semaphore to limit concurrent scans
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent));
        let vulndb = Arc::new(vulndb);
        let scanner = Arc::new(self);
        let scan_options = Arc::new(options);

        // Create futures for scanning each component
        let mut scan_futures = FuturesUnordered::new();

        for component in components {
            let component = component.clone();
            let semaphore = semaphore.clone();
            let vulndb = vulndb.clone();
            let scanner = scanner.clone();
            let scan_options = scan_options.clone();
            let timeout = self.config.component_timeout;

            let scan_future = async move {
                // Acquire semaphore permit to limit concurrency
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|_| AuditError::ScanError {
                        message: "Failed to acquire scan permit".to_string(),
                    })?;

                // Apply timeout to component scan
                let scan_task = scanner.scan_component(&component, &vulndb, &scan_options);

                match tokio::time::timeout(timeout, scan_task).await {
                    Ok(scan_result) => scan_result,
                    Err(_) => Err(AuditError::ScanTimeout {
                        component: format!(
                            "{}@{}",
                            component.identifier.name, component.identifier.version
                        ),
                        timeout_seconds: timeout.as_secs(),
                    }
                    .into()),
                }
            };

            scan_futures.push(scan_future);
        }

        // Collect all scan results
        let mut all_vulnerabilities = Vec::new();
        let mut scan_errors = Vec::new();

        while let Some(scan_result) = scan_futures.next().await {
            match scan_result {
                Ok(matches) => all_vulnerabilities.extend(matches),
                Err(e) => {
                    // Collect errors but continue scanning other components
                    scan_errors.push(e);
                }
            }
        }

        // If we have critical scan errors, fail early
        if !scan_errors.is_empty() && scan_errors.len() == components.len() {
            // All scans failed - return the first error
            return Err(scan_errors.into_iter().next().unwrap());
        }

        // Filter by confidence threshold
        all_vulnerabilities.retain(|v| v.confidence >= options.confidence_threshold);

        // Filter by severity threshold
        all_vulnerabilities.retain(|v| v.vulnerability.severity >= options.severity_threshold);

        let scan_duration = start_time.elapsed();

        let result = ScanResult {
            vulnerabilities: all_vulnerabilities,
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

    /// Check if a version is affected by a vulnerability using robust semver parsing
    fn is_version_affected(&self, version: &str, vulnerability: &Vulnerability) -> bool {
        // Check exact matches in affected versions list first
        if vulnerability
            .affected_versions
            .contains(&version.to_string())
        {
            return true;
        }

        // Check version ranges in affected_versions
        for affected_version in &vulnerability.affected_versions {
            if super::vulndb::parser::is_version_affected(
                version,
                affected_version,
                vulnerability.fixed_versions.first().map(String::as_str),
            ) {
                return true;
            }
        }

        // If no affected versions specified, check against fixed versions
        if vulnerability.affected_versions.is_empty() && !vulnerability.fixed_versions.is_empty() {
            // Try to parse both versions as semver
            match semver::Version::parse(&self.normalize_version(version)) {
                Ok(current_version) => {
                    for fixed_version in &vulnerability.fixed_versions {
                        match semver::Version::parse(&self.normalize_version(fixed_version)) {
                            Ok(fixed_ver) => {
                                if current_version < fixed_ver {
                                    return true;
                                }
                            }
                            Err(_) => {
                                // Fall back to string comparison
                                if version < fixed_version.as_str() {
                                    return true;
                                }
                            }
                        }
                    }
                }
                Err(_) => {
                    // Fall back to string comparison for non-semver versions
                    for fixed_version in &vulnerability.fixed_versions {
                        if version < fixed_version.as_str() {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Normalize a version string to be semver-compatible
    fn normalize_version(&self, version: &str) -> String {
        let trimmed = version.trim();

        // Handle common version formats
        if trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
            let parts: Vec<&str> = trimmed.split('.').collect();
            match parts.len() {
                1 => format!("{}.0.0", parts[0]),
                2 => format!("{}.{}.0", parts[0], parts[1]),
                _ => trimmed.to_string(),
            }
        } else {
            trimmed.to_string()
        }
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

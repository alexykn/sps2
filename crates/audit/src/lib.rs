#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! CVE audit system for spsv2 (Future Implementation)
//!
//! This crate provides offline CVE scanning using embedded SBOMs.
//! Currently implements a foundation with placeholder functionality
//! for future development.

mod sbom_parser;
mod scanner;
mod types;
mod vulndb;

pub use sbom_parser::SbomParser;
pub use scanner::{AuditScanner, ScanOptions, ScanResult};
pub use types::{
    AuditReport, Component, ComponentIdentifier, PackageAudit, Severity, Vulnerability,
    VulnerabilityMatch,
};
pub use vulndb::{VulnDbManager, VulnerabilityDatabase};

use spsv2_errors::Error;
use spsv2_events::EventSender;
use spsv2_state::StateManager;
use spsv2_store::PackageStore;

/// Audit system for CVE scanning
pub struct AuditSystem {
    /// Vulnerability database manager
    vulndb: VulnDbManager,
    /// SBOM parser
    sbom_parser: SbomParser,
    /// Audit scanner
    scanner: AuditScanner,
}

impl AuditSystem {
    /// Create new audit system
    pub fn new(vulndb_path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let vulndb = VulnDbManager::new(vulndb_path)?;
        let sbom_parser = SbomParser::new();
        let scanner = AuditScanner::new();

        Ok(Self {
            vulndb,
            sbom_parser,
            scanner,
        })
    }

    /// Scan all installed packages for vulnerabilities
    pub async fn scan_all_packages(
        &self,
        state_manager: &StateManager,
        store: &PackageStore,
        options: ScanOptions,
        event_sender: Option<EventSender>,
    ) -> Result<AuditReport, Error> {
        // Get all installed packages
        let installed_packages = state_manager.get_installed_packages().await?;

        if let Some(sender) = &event_sender {
            let _ = sender.send(spsv2_events::Event::AuditStarting {
                package_count: installed_packages.len(),
            });
        }

        let mut package_audits = Vec::new();

        for package in &installed_packages {
            let version = package.version();
            let audit = self
                .scan_package(&package.name, &version, store, &options)
                .await?;
            
            let vuln_count = audit.vulnerabilities.len();
            package_audits.push(audit);

            if let Some(sender) = &event_sender {
                let _ = sender.send(spsv2_events::Event::AuditPackageCompleted {
                    package: package.name.clone(),
                    vulnerabilities_found: vuln_count,
                });
            }
        }

        let report = AuditReport::new(package_audits);

        if let Some(sender) = &event_sender {
            let _ = sender.send(spsv2_events::Event::AuditCompleted {
                packages_scanned: installed_packages.len(),
                vulnerabilities_found: report.total_vulnerabilities(),
                critical_count: report.critical_count(),
            });
        }

        Ok(report)
    }

    /// Scan specific package for vulnerabilities
    pub async fn scan_package(
        &self,
        package_name: &str,
        package_version: &spsv2_types::Version,
        store: &PackageStore,
        options: &ScanOptions,
    ) -> Result<PackageAudit, Error> {
        // Get package SBOM from store
        let sbom_data = store
            .get_package_sbom(package_name, package_version)
            .await?;

        // Parse SBOM to extract components
        let components = self.sbom_parser.parse_sbom(&sbom_data)?;

        // Scan components for vulnerabilities
        let vulndb = self.vulndb.get_database().await?;
        let scan_result = self
            .scanner
            .scan_components(&components, &vulndb, options)
            .await?;

        Ok(PackageAudit {
            package_name: package_name.to_string(),
            package_version: package_version.clone(),
            components: components.len(),
            vulnerabilities: scan_result.vulnerabilities,
            scan_timestamp: chrono::Utc::now(),
        })
    }

    /// Update vulnerability database
    pub async fn update_vulnerability_database(&mut self) -> Result<(), Error> {
        self.vulndb.update().await
    }

    /// Check if vulnerability database exists and is recent
    pub async fn check_database_freshness(&self) -> Result<bool, Error> {
        self.vulndb.is_fresh().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_audit_system_creation() {
        let temp = tempdir().unwrap();
        let result = AuditSystem::new(temp.path());

        // Should succeed (even if placeholder implementation)
        assert!(result.is_ok());
    }

    #[test]
    fn test_scan_options() {
        let options = ScanOptions::default();
        assert!(!options.fail_on_critical);
        assert!(options.severity_threshold == Severity::Low);

        let strict_options = ScanOptions::new()
            .with_fail_on_critical(true)
            .with_severity_threshold(Severity::High);

        assert!(strict_options.fail_on_critical);
        assert!(strict_options.severity_threshold == Severity::High);
    }
}

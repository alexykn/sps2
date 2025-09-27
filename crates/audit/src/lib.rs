#![warn(mismatched_lifetime_syntaxes)]
#![deny(clippy::pedantic, unsafe_code)]
// Allow some placeholder implementation issues - will be removed gradually

//! CVE audit system for sps2 (Future Implementation)
//!
//! This crate provides offline CVE scanning using embedded SBOMs.
//! Currently implements a foundation with placeholder functionality
//! for future development.

mod sbom_parser;
mod scanner;
mod types;
mod vulndb;

pub use sbom_parser::SbomParser;
pub use scanner::{AuditScanner, ScanOptions, ScanResult, ScannerConfig};
pub use types::{
    AuditReport, Component, ComponentIdentifier, PackageAudit, Severity, Vulnerability,
    VulnerabilityMatch,
};
pub use vulndb::{DatabaseStatistics, VulnDbManager, VulnerabilityDatabase};

use sps2_errors::Error;
use sps2_events::{AppEvent, AuditEvent, EventEmitter, EventSender};
use sps2_hash::Hash;
use sps2_state::StateManager;
use sps2_store::PackageStore;

/// Audit system for CVE scanning
pub struct AuditSystem {
    /// Vulnerability database manager
    vulndb: VulnDbManager,
    /// SBOM parser
    sbom_parser: SbomParser,
    /// Audit scanner
    scanner: AuditScanner,
    /// Event sender for progress and status updates
    event_sender: Option<EventSender>,
}

impl AuditSystem {
    /// Create new audit system
    ///
    /// # Errors
    ///
    /// Returns an error if the vulnerability database manager cannot be created.
    pub fn new(vulndb_path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let vulndb = VulnDbManager::new(vulndb_path)?;
        let sbom_parser = SbomParser::new();
        let scanner = AuditScanner::new();

        Ok(Self {
            vulndb,
            sbom_parser,
            scanner,
            event_sender: None,
        })
    }

    /// Create new audit system with event sender
    ///
    /// # Errors
    ///
    /// Returns an error if the vulnerability database manager cannot be created.
    pub fn with_events(
        vulndb_path: impl AsRef<std::path::Path>,
        event_sender: EventSender,
    ) -> Result<Self, Error> {
        let vulndb = VulnDbManager::new(vulndb_path)?;
        let sbom_parser = SbomParser::new();
        let scanner = AuditScanner::new();

        Ok(Self {
            vulndb,
            sbom_parser,
            scanner,
            event_sender: Some(event_sender),
        })
    }

    /// Scan all installed packages for vulnerabilities
    ///
    /// # Errors
    ///
    /// Returns an error if the list of installed packages cannot be retrieved
    /// or if the scan of any package fails.
    pub async fn scan_all_packages(
        &self,
        state_manager: &StateManager,
        store: &PackageStore,
        options: ScanOptions,
    ) -> Result<AuditReport, Error> {
        // Get all installed packages
        let installed_packages = state_manager.get_installed_packages().await?;

        if let Some(sender) = self.event_sender() {
            sender.emit(AppEvent::Audit(AuditEvent::ScanStarted {
                package_count: installed_packages.len(),
            }));
        }

        let mut package_audits = Vec::new();

        for package in &installed_packages {
            let version = package.version();
            let package_hash = Hash::from_hex(&package.hash).map_err(|e| {
                sps2_errors::AuditError::InvalidData {
                    message: format!("Invalid package hash: {e}"),
                }
            })?;

            let audit = self
                .scan_package(&package.name, &version, &package_hash, store, &options)
                .await?;

            let vuln_count = audit.vulnerabilities.len();
            package_audits.push(audit);

            if let Some(sender) = self.event_sender() {
                sender.emit(AppEvent::Audit(AuditEvent::ScanPackageCompleted {
                    package: package.name.clone(),
                    vulnerabilities_found: vuln_count,
                }));
            }
        }

        let report = AuditReport::new(package_audits);

        if let Some(sender) = self.event_sender() {
            sender.emit(AppEvent::Audit(AuditEvent::ScanCompleted {
                packages_scanned: installed_packages.len(),
                vulnerabilities_found: report.total_vulnerabilities(),
                critical_count: report.critical_count(),
            }));
        }

        Ok(report)
    }

    /// Scan specific package for vulnerabilities
    ///
    /// # Errors
    ///
    /// Returns an error if the package SBOM cannot be retrieved, the SBOM
    /// cannot be parsed, or the vulnerability scan fails.
    pub async fn scan_package(
        &self,
        package_name: &str,
        package_version: &sps2_types::Version,
        package_hash: &Hash,
        store: &PackageStore,
        options: &ScanOptions,
    ) -> Result<PackageAudit, Error> {
        // Get package SBOM from store
        let sbom_data = store.get_package_sbom(package_hash).await?;

        // Parse SBOM to extract components
        let components = self.sbom_parser.parse_sbom(&sbom_data)?;

        // Scan components for vulnerabilities
        let vulndb = self.vulndb.get_database()?;
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
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn update_vulnerability_database(&mut self) -> Result<(), Error> {
        self.vulndb.update().await
    }

    /// Update vulnerability database with event reporting
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn update_vulnerability_database_with_events(&mut self) -> Result<(), Error> {
        let event_sender = self.event_sender.as_ref();
        self.vulndb.update_with_events(event_sender).await
    }

    /// Check if vulnerability database exists and is recent
    ///
    /// # Errors
    ///
    /// Returns an error if the database freshness check fails.
    pub async fn check_database_freshness(&self) -> Result<bool, Error> {
        self.vulndb.is_fresh().await
    }
}

impl EventEmitter for AuditSystem {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

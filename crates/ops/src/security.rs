//! Security and Vulnerability Management Operations

use crate::{OpsCtx, VulnDbStats};
use sps2_errors::{Error, OpsError};
use sps2_events::{Event, EventEmitter};

/// Update vulnerability database
///
/// # Errors
///
/// Returns an error if the vulnerability database update fails.
pub async fn update_vulndb(_ctx: &OpsCtx) -> Result<String, Error> {
    // Initialize vulnerability database manager
    let mut vulndb = sps2_audit::VulnDbManager::new(sps2_audit::VulnDbManager::default_path())?;

    // Initialize if needed
    vulndb.initialize().await?;

    // Update the database from all sources
    vulndb.update().await?;

    Ok("Vulnerability database updated successfully".to_string())
}

/// Get vulnerability database statistics
///
/// # Errors
///
/// Returns an error if the vulnerability database cannot be accessed.
pub async fn vulndb_stats(_ctx: &OpsCtx) -> Result<VulnDbStats, Error> {
    // Initialize vulnerability database manager
    let mut vulndb = sps2_audit::VulnDbManager::new(sps2_audit::VulnDbManager::default_path())?;

    // Initialize if needed
    vulndb.initialize().await?;

    // Get database
    let db = vulndb.get_database()?;

    // Get statistics
    let stats = db.get_statistics().await?;

    // Get database file size
    let db_path = sps2_audit::VulnDbManager::default_path();
    let metadata = tokio::fs::metadata(&db_path).await?;
    let database_size = metadata.len();

    Ok(VulnDbStats {
        vulnerability_count: stats.vulnerability_count,
        last_updated: stats.last_updated,
        database_size,
        severity_breakdown: stats.severity_breakdown,
    })
}

/// Audit packages for vulnerabilities
///
/// # Errors
///
/// Returns an error if the audit scan fails.
pub async fn audit(
    ctx: &OpsCtx,
    package_name: Option<&str>,
    fail_on_critical: bool,
    severity_threshold: sps2_audit::Severity,
) -> Result<sps2_audit::AuditReport, Error> {
    // Create audit system
    let audit_system = sps2_audit::AuditSystem::with_events(
        sps2_audit::VulnDbManager::default_path(),
        ctx.tx.clone(),
    )?;

    // Configure scan options
    let scan_options = sps2_audit::ScanOptions::new()
        .with_fail_on_critical(fail_on_critical)
        .with_severity_threshold(severity_threshold);

    // Run audit based on whether a specific package is requested
    let report = if let Some(name) = package_name {
        // Scan specific package
        let installed_packages = ctx.state.get_installed_packages().await?;
        let package = installed_packages
            .iter()
            .find(|pkg| pkg.name == name)
            .ok_or_else(|| OpsError::PackageNotFound {
                package: name.to_string(),
            })?;

        ctx.emit_event(Event::AuditStarting { package_count: 1 });

        let package_hash =
            sps2_hash::Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
                message: format!("Invalid package hash: {e}"),
            })?;

        let package_audit = audit_system
            .scan_package(
                &package.name,
                &package.version(),
                &package_hash,
                &ctx.store,
                &scan_options,
            )
            .await?;

        let vuln_count = package_audit.vulnerabilities.len();
        ctx.emit_event(Event::AuditPackageCompleted {
            package: package.name.clone(),
            vulnerabilities_found: vuln_count,
        });

        let report = sps2_audit::AuditReport::new(vec![package_audit]);

        ctx.emit_event(Event::AuditCompleted {
            packages_scanned: 1,
            vulnerabilities_found: report.total_vulnerabilities(),
            critical_count: report.critical_count(),
        });

        report
    } else {
        // Scan all packages
        audit_system
            .scan_all_packages(&ctx.state, &ctx.store, scan_options)
            .await?
    };

    Ok(report)
}

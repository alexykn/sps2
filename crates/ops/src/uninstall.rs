//! Uninstall command implementation
//!
//! Handles package removal with dependency checking.
//! Delegates to `sps2_install` crate for the actual uninstall logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::{Error, OpsError};
use sps2_events::Event;
use sps2_install::{InstallConfig, Installer, UninstallContext};
use std::time::Instant;

/// Uninstall packages (delegates to install crate)
///
/// # Errors
///
/// Returns an error if:
/// - No packages are specified
/// - Package removal would break dependencies
/// - Uninstallation fails
pub async fn uninstall(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    let start = Instant::now();

    if package_names.is_empty() {
        return Err(OpsError::NoPackagesSpecified.into());
    }

    ctx.tx
        .send(Event::UninstallStarting {
            packages: package_names.to_vec(),
        })
        .ok();

    // Create installer
    let config = InstallConfig::default();
    let mut installer = Installer::new(
        config,
        ctx.resolver.clone(),
        ctx.state.clone(),
        ctx.store.clone(),
    );

    // Build uninstall context
    let mut uninstall_context = UninstallContext::new().with_event_sender(ctx.tx.clone());

    for package_name in package_names {
        uninstall_context = uninstall_context.add_package(package_name.clone());
    }

    // Execute uninstallation
    let result = installer.uninstall(uninstall_context).await?;

    // Convert to report format
    let report = InstallReport {
        installed: Vec::new(), // No packages installed during uninstall
        updated: Vec::new(),   // No packages updated during uninstall
        removed: result
            .removed_packages
            .iter()
            .map(|pkg| crate::PackageChange {
                name: pkg.name.clone(),
                from_version: Some(pkg.version.clone()),
                to_version: None,
                size: None,
            })
            .collect(),
        state_id: result.state_id,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
    };

    ctx.tx
        .send(Event::UninstallCompleted {
            packages: result
                .removed_packages
                .iter()
                .map(|pkg| pkg.name.clone())
                .collect(),
            state_id: result.state_id,
        })
        .ok();

    Ok(report)
}

//! Upgrade command implementation
//!
//! Handles package upgrades, ignoring version constraints to get latest versions.
//! Delegates to `sps2_install` crate for the actual upgrade logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::Error;
use sps2_events::Event;
use sps2_install::{InstallConfig, Installer, UpdateContext};
use sps2_types::Version;
use std::time::Instant;

/// Upgrade packages (delegates to install crate)
///
/// # Errors
///
/// Returns an error if:
/// - No packages are installed or specified
/// - Upgrade resolution fails
/// - Installation of upgrades fails
pub async fn upgrade(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    let start = Instant::now();

    ctx.tx
        .send(Event::UpgradeStarting {
            packages: if package_names.is_empty() {
                vec!["all".to_string()]
            } else {
                package_names.to_vec()
            },
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

    // Build update context with upgrade mode
    let mut update_context = UpdateContext::new()
        .with_upgrade(true) // Upgrade mode (ignore upper bounds)
        .with_event_sender(ctx.tx.clone());

    for package_name in package_names {
        update_context = update_context.add_package(package_name.clone());
    }

    // Get currently installed packages before upgrade to track from_version
    let installed_before = ctx.state.get_installed_packages().await?;
    let installed_map: std::collections::HashMap<String, Version> = installed_before
        .iter()
        .map(|pkg| (pkg.name.clone(), pkg.version()))
        .collect();

    // Execute upgrade
    let result = installer.update(update_context).await?;

    // Convert to report format
    let report = InstallReport {
        installed: result
            .installed_packages
            .iter()
            .map(|pkg| crate::PackageChange {
                name: pkg.name.clone(),
                from_version: None,
                to_version: Some(pkg.version.clone()),
                size: None,
            })
            .collect(),
        updated: result
            .updated_packages
            .iter()
            .map(|pkg| crate::PackageChange {
                name: pkg.name.clone(),
                from_version: installed_map.get(&pkg.name).cloned(),
                to_version: Some(pkg.version.clone()),
                size: None,
            })
            .collect(),
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
        .send(Event::UpgradeCompleted {
            packages: result
                .updated_packages
                .iter()
                .map(|pkg| pkg.name.clone())
                .collect(),
            state_id: result.state_id,
        })
        .ok();

    Ok(report)
}

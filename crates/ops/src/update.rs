//! Update command implementation
//!
//! Handles package updates, respecting version constraints.
//! Delegates to `sps2_install` crate for the actual update logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::Error;
use sps2_events::{
    events::{UpdateOperationType, UpdateResult},
    AppEvent, EventEmitter, UpdateEvent,
};
use sps2_install::{InstallConfig, Installer, UpdateContext};
use sps2_types::Version;
use std::time::Instant;

/// Update packages (delegates to install crate)
///
/// # Errors
///
/// Returns an error if:
/// - No packages are installed or specified
/// - Update resolution fails
/// - Installation of updates fails
pub async fn update(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    let start = Instant::now();

    ctx.emit(AppEvent::Update(UpdateEvent::Started {
        operation_type: UpdateOperationType::Update,
        packages_specified: if package_names.is_empty() {
            vec!["all".to_string()]
        } else {
            package_names.to_vec()
        },
        check_all_packages: package_names.is_empty(),
        ignore_constraints: false,
    }));

    // Create installer
    let config = InstallConfig::default();
    let mut installer = Installer::new(
        config,
        ctx.resolver.clone(),
        ctx.state.clone(),
        ctx.store.clone(),
    );

    // Build update context
    let mut update_context = UpdateContext::new()
        .with_upgrade(false) // Update mode (respect upper bounds)
        .with_event_sender(ctx.tx.clone());

    for package_name in package_names {
        update_context = update_context.add_package(package_name.clone());
    }

    // Get currently installed packages before update to track from_version
    let installed_before = ctx.state.get_installed_packages().await?;
    let installed_map: std::collections::HashMap<String, Version> = installed_before
        .iter()
        .map(|pkg| (pkg.name.clone(), pkg.version()))
        .collect();

    // Execute update
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

    ctx.emit(AppEvent::Update(UpdateEvent::Completed {
        operation_type: UpdateOperationType::Update,
        packages_updated: result
            .updated_packages
            .iter()
            .map(|pkg| UpdateResult {
                package: pkg.name.clone(),
                from_version: installed_map
                    .get(&pkg.name)
                    .cloned()
                    .unwrap_or_else(|| pkg.version.clone()),
                to_version: pkg.version.clone(),
                update_type: sps2_events::events::PackageUpdateType::Minor, // TODO: Determine actual update type
                duration: std::time::Duration::from_secs(30), // TODO: Track actual duration per package
                size_change: 0,                               // TODO: Calculate actual size change
            })
            .collect(),
        packages_unchanged: result
            .installed_packages
            .iter()
            .map(|pkg| pkg.name.clone())
            .collect(),
        total_duration: start.elapsed(),
        space_difference: 0, // TODO: Calculate actual space difference
    }));

    Ok(report)
}

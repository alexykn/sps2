//! Upgrade command implementation
//!
//! Handles package upgrades, ignoring version constraints to get latest versions.
//! Delegates to `sps2_install` crate for the actual upgrade logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::Error;
use sps2_events::{
    events::{LifecyclePackageUpdateType, LifecycleUpdateOperation, LifecycleUpdateResult},
    patterns::UpdateProgressConfig,
    AppEvent, EventEmitter, FailureContext, GeneralEvent, LifecycleEvent, ProgressEvent,
    ProgressManager,
};
use sps2_install::{InstallConfig, Installer, UpdateContext};
use sps2_types::{PackageSpec, Version};
use std::time::Instant;
use uuid::Uuid;

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

    let _correlation = ctx.push_correlation_for_packages("upgrade", package_names);

    // Check mode: preview what would be upgraded
    if ctx.check_mode {
        return preview_upgrade(ctx, package_names).await;
    }

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

    let total_targets = if package_names.is_empty() {
        installed_before.len()
    } else {
        package_names.len()
    };
    let requested_packages: Vec<String> = if package_names.is_empty() {
        Vec::new()
    } else {
        package_names.to_vec()
    };

    let progress_manager = ProgressManager::new();
    let update_config = UpdateProgressConfig {
        operation_name: "Upgrading packages".to_string(),
        package_count: total_targets as u64,
        is_upgrade: true,
    };
    let progress_id = progress_manager.create_update_tracker(update_config);
    let correlation = ctx.current_correlation();
    progress_manager.emit_started(&progress_id, ctx, correlation.as_deref());

    ctx.emit(AppEvent::Lifecycle(LifecycleEvent::update_started(
        LifecycleUpdateOperation::Upgrade,
        requested_packages.clone(),
        total_targets,
    )));

    // Execute upgrade
    let result = installer.update(update_context).await.inspect_err(|e| {
        let failure = FailureContext::from_error(e);
        ctx.emit_operation_failed("upgrade", failure.clone());

        ctx.emit(AppEvent::Progress(ProgressEvent::Failed {
            id: progress_id.clone(),
            failure: failure.clone(),
            completed_items: 0,
            partial_duration: start.elapsed(),
        }));

        ctx.emit(AppEvent::Lifecycle(LifecycleEvent::update_failed(
            LifecycleUpdateOperation::Upgrade,
            Vec::new(),
            if requested_packages.is_empty() {
                Vec::new()
            } else {
                requested_packages.clone()
            },
            failure,
        )));
    })?;

    let report = create_upgrade_report(
        &result,
        &installed_map,
        start,
        ctx,
        UpdateReportContext {
            progress_id: &progress_id,
            progress_manager: &progress_manager,
            total_targets,
            operation: LifecycleUpdateOperation::Upgrade,
        },
    );
    Ok(report)
}

struct UpdateReportContext<'a> {
    progress_id: &'a str,
    progress_manager: &'a ProgressManager,
    total_targets: usize,
    operation: LifecycleUpdateOperation,
}

fn create_upgrade_report(
    result: &sps2_install::InstallResult,
    installed_map: &std::collections::HashMap<String, sps2_types::Version>,
    start: std::time::Instant,
    ctx: &OpsCtx,
    context: UpdateReportContext<'_>,
) -> InstallReport {
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

    context
        .progress_manager
        .complete_operation(context.progress_id, ctx);

    let updated_results: Vec<LifecycleUpdateResult> = result
        .updated_packages
        .iter()
        .map(|pkg| LifecycleUpdateResult {
            package: pkg.name.clone(),
            from_version: installed_map
                .get(&pkg.name)
                .cloned()
                .unwrap_or_else(|| pkg.version.clone()),
            to_version: pkg.version.clone(),
            update_type: LifecyclePackageUpdateType::Major, // TODO: Determine actual update type
            duration: std::time::Duration::from_secs(30), // TODO: Track actual duration per package
            size_change: 0,                               // TODO: Calculate actual space difference
        })
        .collect();

    let skipped = context
        .total_targets
        .saturating_sub(updated_results.len())
        .saturating_sub(result.installed_packages.len());

    ctx.emit(AppEvent::Lifecycle(LifecycleEvent::update_completed(
        context.operation,
        updated_results,
        skipped,
        start.elapsed(),
        0,
    )));

    report
}

/// Preview what would be upgraded without executing
#[allow(clippy::too_many_lines)]
async fn preview_upgrade(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    use std::collections::HashMap;

    // Get currently installed packages
    let current_packages = ctx.state.get_installed_packages().await?;

    // Determine packages to check for upgrades
    let packages_to_check = if package_names.is_empty() {
        // Check all packages
        current_packages.clone()
    } else {
        // Check specified packages
        current_packages
            .iter()
            .filter(|pkg| package_names.contains(&pkg.name))
            .cloned()
            .collect()
    };

    let mut preview_updated = Vec::new();
    let mut packages_up_to_date = Vec::new();
    let mut packages_not_found = Vec::new();

    // Check for packages that were specified but not found
    if !package_names.is_empty() {
        for package_name in package_names {
            if !current_packages.iter().any(|pkg| &pkg.name == package_name) {
                packages_not_found.push(package_name.clone());
            }
        }
    }

    // Report packages that are not installed
    for package_name in &packages_not_found {
        ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
            operation: "upgrade".to_string(),
            action: format!("Package {package_name} is not installed"),
            details: HashMap::from([
                ("status".to_string(), "not_installed".to_string()),
                ("action".to_string(), "skip".to_string()),
            ]),
        }));
    }

    // Check each installed package for available upgrades
    for package_id in &packages_to_check {
        // Create a spec that allows any version >= 0.0.0 (upgrade mode ignores upper bounds)
        let spec = PackageSpec::parse(&format!("{}>=0.0.0", package_id.name))?;

        // Create resolution context for this package
        let mut resolution_context = sps2_resolver::ResolutionContext::new();
        resolution_context = resolution_context.add_runtime_dep(spec);

        // Resolve to see what version would be installed
        match ctx.resolver.resolve_with_sat(resolution_context).await {
            Ok(resolution_result) => {
                // Check if any resolved package is newer than current
                let mut found_upgrade = false;

                for (resolved_id, node) in &resolution_result.nodes {
                    if resolved_id.name == package_id.name {
                        match resolved_id.version.cmp(&package_id.version()) {
                            std::cmp::Ordering::Greater => {
                                // Upgrade available
                                let change_type = determine_upgrade_type(
                                    &package_id.version(),
                                    &resolved_id.version,
                                );

                                ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                                    operation: "upgrade".to_string(),
                                    action: format!(
                                        "Would upgrade {} {} → {}",
                                        package_id.name, package_id.version, resolved_id.version
                                    ),
                                    details: HashMap::from([
                                        (
                                            "current_version".to_string(),
                                            package_id.version.to_string(),
                                        ),
                                        (
                                            "new_version".to_string(),
                                            resolved_id.version.to_string(),
                                        ),
                                        ("change_type".to_string(), change_type),
                                        (
                                            "source".to_string(),
                                            match node.action {
                                                sps2_resolver::NodeAction::Download => {
                                                    "repository".to_string()
                                                }
                                                sps2_resolver::NodeAction::Local => {
                                                    "local file".to_string()
                                                }
                                            },
                                        ),
                                    ]),
                                }));

                                preview_updated.push(crate::PackageChange {
                                    name: package_id.name.clone(),
                                    from_version: Some(package_id.version()),
                                    to_version: Some(resolved_id.version.clone()),
                                    size: None,
                                });

                                found_upgrade = true;
                            }
                            std::cmp::Ordering::Equal => {
                                // Already up to date
                                packages_up_to_date.push(package_id.name.clone());
                            }
                            std::cmp::Ordering::Less => {}
                        }
                        break;
                    }
                }

                if !found_upgrade
                    && !packages_up_to_date
                        .iter()
                        .any(|name| name == &package_id.name)
                {
                    // No upgrade found, package is up to date
                    packages_up_to_date.push(package_id.name.clone());
                }
            }
            Err(_) => {
                // Resolution failed - package might not be available in repository
                ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                    operation: "upgrade".to_string(),
                    action: format!("Cannot check upgrades for {}", package_id.name),
                    details: HashMap::from([
                        (
                            "current_version".to_string(),
                            package_id.version.to_string(),
                        ),
                        ("status".to_string(), "resolution_failed".to_string()),
                        (
                            "reason".to_string(),
                            "package not found in repository".to_string(),
                        ),
                    ]),
                }));
            }
        }
    }

    // Show packages that are already up to date (only if there are upgrades available)
    if !preview_updated.is_empty() {
        for package_name in &packages_up_to_date {
            if let Some(package_id) = current_packages
                .iter()
                .find(|pkg| &pkg.name == package_name)
            {
                ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                    operation: "upgrade".to_string(),
                    action: format!(
                        "{}:{} is already at latest version",
                        package_id.name, package_id.version
                    ),
                    details: HashMap::from([
                        ("version".to_string(), package_id.version.to_string()),
                        ("status".to_string(), "up_to_date".to_string()),
                    ]),
                }));
            }
        }
    }

    // Emit summary
    let total_changes = preview_updated.len();
    let mut categories = HashMap::new();
    categories.insert("packages_upgraded".to_string(), preview_updated.len());
    categories.insert("packages_up_to_date".to_string(), packages_up_to_date.len());
    if !packages_not_found.is_empty() {
        categories.insert("packages_not_found".to_string(), packages_not_found.len());
    }

    ctx.emit(AppEvent::General(GeneralEvent::CheckModeSummary {
        operation: "upgrade".to_string(),
        total_changes,
        categories,
    }));

    // Return preview report (no actual state changes)
    Ok(InstallReport {
        installed: Vec::new(),
        updated: preview_updated,
        removed: Vec::new(),
        state_id: Uuid::nil(), // No state change in preview
        duration_ms: 0,
    })
}

/// Determine the type of upgrade based on version changes
fn determine_upgrade_type(from: &Version, to: &Version) -> String {
    if from.major != to.major {
        "major".to_string()
    } else if from.minor != to.minor {
        "minor".to_string()
    } else if from.patch != to.patch {
        "patch".to_string()
    } else {
        "prerelease".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_upgrade_types() {
        let mut v1 = Version::new(1, 0, 0);
        let mut v2 = Version::new(2, 0, 0);
        assert_eq!(determine_upgrade_type(&v1, &v2), "major");

        v1 = Version::new(1, 1, 0);
        v2 = Version::new(1, 2, 0);
        assert_eq!(determine_upgrade_type(&v1, &v2), "minor");

        v1 = Version::new(1, 1, 1);
        v2 = Version::new(1, 1, 2);
        assert_eq!(determine_upgrade_type(&v1, &v2), "patch");

        v1 = Version::new(1, 1, 1);
        v2 = Version::new(1, 1, 1);
        assert_eq!(determine_upgrade_type(&v1, &v2), "prerelease");
    }
}

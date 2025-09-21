//! Update command implementation
//!
//! Handles package updates, respecting version constraints.
//! Delegates to `sps2_install` crate for the actual update logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::Error;
use sps2_events::{
    events::{UpdateOperationType, UpdateResult},
    patterns::UpdateProgressConfig,
    AppEvent, EventEmitter, FailureContext, GeneralEvent, ProgressEvent, ProgressManager,
    UpdateEvent,
};
use sps2_install::{InstallConfig, Installer, UpdateContext};
use sps2_types::{PackageSpec, Version};
use std::time::Instant;
use uuid::Uuid;

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

    let _correlation = ctx.push_correlation_for_packages("update", package_names);

    // Check mode: preview what would be updated
    if ctx.check_mode {
        return preview_update(ctx, package_names).await;
    }

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
        operation_name: "Updating packages".to_string(),
        package_count: total_targets as u64,
        is_upgrade: false,
    };
    let progress_id = progress_manager.create_update_tracker(update_config);
    let correlation = ctx.current_correlation();
    progress_manager.emit_started(&progress_id, ctx, correlation.as_deref());

    ctx.emit(AppEvent::Update(UpdateEvent::Started {
        operation: UpdateOperationType::Update,
        requested: requested_packages.clone(),
        total_targets,
    }));

    // Execute update
    let result = installer.update(update_context).await.inspect_err(|e| {
        let failure = FailureContext::from_error(e);
        ctx.emit_operation_failed("update", failure.clone());

        ctx.emit(AppEvent::Progress(ProgressEvent::Failed {
            id: progress_id.clone(),
            failure: failure.clone(),
            completed_items: 0,
            partial_duration: start.elapsed(),
        }));

        ctx.emit(AppEvent::Update(UpdateEvent::Failed {
            operation: UpdateOperationType::Update,
            failure,
            updated: Vec::new(),
            failed: if requested_packages.is_empty() {
                Vec::new()
            } else {
                requested_packages.clone()
            },
        }));
    })?;

    let report = create_update_report(
        &result,
        &installed_map,
        start,
        ctx,
        UpdateReportContext {
            progress_id: &progress_id,
            progress_manager: &progress_manager,
            total_targets,
            operation: UpdateOperationType::Update,
        },
    );
    Ok(report)
}

struct UpdateReportContext<'a> {
    progress_id: &'a str,
    progress_manager: &'a ProgressManager,
    total_targets: usize,
    operation: UpdateOperationType,
}

fn create_update_report(
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

    let updated_results: Vec<UpdateResult> = result
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
        .collect();

    let skipped = context
        .total_targets
        .saturating_sub(updated_results.len())
        .saturating_sub(result.installed_packages.len());

    ctx.emit(AppEvent::Update(UpdateEvent::Completed {
        operation: context.operation,
        updated: updated_results,
        skipped,
        duration: start.elapsed(),
        size_difference: 0, // TODO: Calculate actual space difference
    }));

    report
}

/// Preview what would be updated without executing
#[allow(clippy::too_many_lines)]
async fn preview_update(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    use std::collections::HashMap;

    // Get currently installed packages
    let current_packages = ctx.state.get_installed_packages().await?;

    // Determine packages to check for updates
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
            operation: "update".to_string(),
            action: format!("Package {package_name} is not installed"),
            details: HashMap::from([
                ("status".to_string(), "not_installed".to_string()),
                ("action".to_string(), "skip".to_string()),
            ]),
        }));
    }

    // Check each installed package for available updates
    for package_id in &packages_to_check {
        // Create a compatible release spec for update mode (respects upper bounds)
        let spec = match PackageSpec::parse(&format!("{}~={}", package_id.name, package_id.version))
        {
            Ok(spec) => spec,
            Err(_) => {
                // Fallback to any version if parsing fails
                PackageSpec::parse(&format!("{}>=0.0.0", package_id.name))?
            }
        };

        // Create resolution context for this package
        let mut resolution_context = sps2_resolver::ResolutionContext::new();
        resolution_context = resolution_context.add_runtime_dep(spec);

        // Resolve to see what version would be installed
        match ctx.resolver.resolve_with_sat(resolution_context).await {
            Ok(resolution_result) => {
                // Check if any resolved package is newer than current
                let mut found_update = false;

                for (resolved_id, node) in &resolution_result.nodes {
                    if resolved_id.name == package_id.name {
                        match resolved_id.version.cmp(&package_id.version()) {
                            std::cmp::Ordering::Greater => {
                                // Update available
                                let change_type = determine_update_type(
                                    &package_id.version(),
                                    &resolved_id.version,
                                );

                                ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                                    operation: "update".to_string(),
                                    action: format!(
                                        "Would update {} {} → {}",
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

                                found_update = true;
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

                if !found_update
                    && !packages_up_to_date
                        .iter()
                        .any(|name| name == &package_id.name)
                {
                    // No update found, package is up to date
                    packages_up_to_date.push(package_id.name.clone());
                }
            }
            Err(_) => {
                // Resolution failed - package might not be available in repository
                ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                    operation: "update".to_string(),
                    action: format!("Cannot check updates for {}", package_id.name),
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

    // Show packages that are already up to date (only if there are updates available)
    if !preview_updated.is_empty() {
        for package_name in &packages_up_to_date {
            if let Some(package_id) = current_packages
                .iter()
                .find(|pkg| &pkg.name == package_name)
            {
                ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                    operation: "update".to_string(),
                    action: format!(
                        "{}:{} is already up to date",
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
    categories.insert("packages_updated".to_string(), preview_updated.len());
    categories.insert("packages_up_to_date".to_string(), packages_up_to_date.len());
    if !packages_not_found.is_empty() {
        categories.insert("packages_not_found".to_string(), packages_not_found.len());
    }

    ctx.emit(AppEvent::General(GeneralEvent::CheckModeSummary {
        operation: "update".to_string(),
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

/// Determine the type of update based on version changes
fn determine_update_type(from: &Version, to: &Version) -> String {
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

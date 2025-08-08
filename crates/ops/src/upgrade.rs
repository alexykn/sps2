//! Upgrade command implementation
//!
//! Handles package upgrades, ignoring version constraints to get latest versions.
//! Delegates to `sps2_install` crate for the actual upgrade logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::Error;
use sps2_events::{
    events::{UpdateOperationType, UpdateResult},
    AppEvent, EventEmitter, GeneralEvent, UpdateEvent,
};
use sps2_events::{patterns::UpdateProgressConfig, ProgressManager};
use sps2_guard::{OperationResult as GuardOperationResult, PackageChange as GuardPackageChange};
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

    // Check mode: preview what would be upgraded
    if ctx.check_mode {
        return preview_upgrade(ctx, package_names).await;
    }

    let progress_manager = ProgressManager::new();
    let update_config = UpdateProgressConfig {
        operation_name: "Upgrading packages".to_string(),
        package_count: package_names.len() as u64,
        is_upgrade: true,
    };
    let progress_id = progress_manager.create_update_tracker(update_config);

    ctx.emit(AppEvent::Update(UpdateEvent::Started {
        operation_type: UpdateOperationType::Upgrade,
        packages_specified: if package_names.is_empty() {
            vec!["all".to_string()]
        } else {
            package_names.to_vec()
        },
        check_all_packages: package_names.is_empty(),
        ignore_constraints: true,
    }));

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

    let report = create_upgrade_report(
        &result,
        &installed_map,
        start,
        ctx,
        &progress_id,
        &progress_manager,
    );
    Ok(report)
}

fn create_upgrade_report(
    result: &sps2_install::InstallResult,
    installed_map: &std::collections::HashMap<String, sps2_types::Version>,
    start: std::time::Instant,
    ctx: &OpsCtx,
    progress_id: &str,
    progress_manager: &ProgressManager,
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

    progress_manager.complete_operation(progress_id, &ctx.tx);

    ctx.emit(AppEvent::Update(UpdateEvent::Completed {
        operation_type: UpdateOperationType::Upgrade,
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
                update_type: sps2_events::events::PackageUpdateType::Major, // TODO: Determine actual update type
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

    // Show packages that are already up to date
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

/// Convert `InstallReport` to `GuardOperationResult` for upgrade operations
fn create_guard_operation_result_for_upgrade(report: &InstallReport) -> GuardOperationResult {
    GuardOperationResult {
        installed: report
            .installed
            .iter()
            .map(|pkg| GuardPackageChange {
                name: pkg.name.clone(),
                from_version: pkg
                    .from_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                to_version: pkg
                    .to_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                size: pkg.size,
            })
            .collect(),
        updated: report
            .updated
            .iter()
            .map(|pkg| GuardPackageChange {
                name: pkg.name.clone(),
                from_version: pkg
                    .from_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                to_version: pkg
                    .to_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                size: pkg.size,
            })
            .collect(),
        removed: report
            .removed
            .iter()
            .map(|pkg| GuardPackageChange {
                name: pkg.name.clone(),
                from_version: pkg
                    .from_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                to_version: pkg
                    .to_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                size: pkg.size,
            })
            .collect(),
        state_id: report.state_id,
        duration_ms: report.duration_ms,
        modified_directories: vec![
            std::path::PathBuf::from(sps2_config::fixed_paths::LIVE_DIR),
            std::path::PathBuf::from(sps2_config::fixed_paths::BIN_DIR),
            std::path::PathBuf::from(format!("{}/lib", sps2_config::fixed_paths::LIVE_DIR)),
            std::path::PathBuf::from(format!("{}/share", sps2_config::fixed_paths::LIVE_DIR)),
        ],
        install_triggered: false, // Standard upgrade operation
    }
}

/// Upgrade packages with state verification enabled
///
/// This wrapper uses the advanced `GuardedOperation` pattern providing:
/// - Cache warming before operation
/// - Operation-specific verification scoping for old→new transition verification
/// - Pre-verification of existing packages before upgrade
/// - Post-verification ensuring clean old→new package transitions
/// - Progressive verification when appropriate
/// - Smart cache invalidation after operation
///
/// # Errors
///
/// Returns an error if:
/// - Pre-upgrade verification fails (when `fail_on_discrepancy` is true)
/// - Upgrade operation fails
/// - Post-upgrade verification fails (when `fail_on_discrepancy` is true)
pub async fn upgrade_with_verification(
    ctx: &OpsCtx,
    package_names: &[String],
) -> Result<InstallReport, Error> {
    let package_names_vec = package_names.iter().map(ToString::to_string).collect();

    ctx.guarded_upgrade(package_names_vec)
        .execute(|| async {
            let report = upgrade(ctx, package_names).await?;
            let guard_result = create_guard_operation_result_for_upgrade(&report);
            Ok((report, guard_result))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpsContextBuilder;
    use sps2_builder::Builder;
    use sps2_config::Config;
    use sps2_index::IndexManager;
    use sps2_net::NetClient;
    use sps2_resolver::Resolver;
    use sps2_state::StateManager;
    use sps2_store::PackageStore;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_upgrade_with_verification_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        let state = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir.clone());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let config = Config::default(); // Verification disabled by default

        let index = IndexManager::new(&store_dir);
        let net = NetClient::new(sps2_net::NetConfig::default()).unwrap();
        let resolver = Resolver::with_events(index.clone(), tx.clone());
        let builder = Builder::new();

        let ctx = OpsContextBuilder::new()
            .with_state(state)
            .with_store(store)
            .with_event_sender(tx)
            .with_config(config)
            .with_index(index)
            .with_net(net)
            .with_resolver(resolver)
            .with_builder(builder)
            .build()
            .unwrap();

        // Test that upgrade_with_verification works without verification enabled
        let result = upgrade_with_verification(&ctx, &["test-package".to_string()]).await;

        // Should fail with upgrade error, not verification error (no packages installed)
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("verification"),
                "Should not fail due to verification"
            );
        }
    }

    #[tokio::test]
    async fn test_upgrade_with_verification_enabled() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        let state = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir.clone());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let mut config = Config::default();
        config.verification.enabled = true;
        config.verification.level = "standard".to_string();

        let index = IndexManager::new(&store_dir);
        let net = NetClient::new(sps2_net::NetConfig::default()).unwrap();
        let resolver = Resolver::with_events(index.clone(), tx.clone());
        let builder = Builder::new();

        let mut ctx = OpsContextBuilder::new()
            .with_state(state)
            .with_store(store)
            .with_event_sender(tx)
            .with_config(config)
            .with_index(index)
            .with_net(net)
            .with_resolver(resolver)
            .with_builder(builder)
            .build()
            .unwrap();

        // Initialize the guard
        ctx.initialize_guard().unwrap();

        // Test that upgrade_with_verification runs verification
        let result = upgrade_with_verification(&ctx, &["test-package".to_string()]).await;

        // Should fail with upgrade error (package not found), not verification error
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("verification failed"),
                "Should not fail due to verification on empty state"
            );
        }
    }

    #[tokio::test]
    async fn test_create_guard_operation_result_for_upgrade() {
        let report = InstallReport {
            installed: vec![],
            updated: vec![crate::PackageChange {
                name: "test-package".to_string(),
                from_version: Some(sps2_types::Version::parse("1.0.0").unwrap()),
                to_version: Some(sps2_types::Version::parse("2.0.0").unwrap()),
                size: Some(2048),
            }],
            removed: vec![],
            state_id: uuid::Uuid::new_v4(),
            duration_ms: 150,
        };

        let guard_result = create_guard_operation_result_for_upgrade(&report);

        assert_eq!(guard_result.installed.len(), 0);
        assert_eq!(guard_result.updated.len(), 1);
        assert_eq!(guard_result.removed.len(), 0);
        assert_eq!(guard_result.updated[0].name, "test-package");
        assert_eq!(
            guard_result.updated[0].from_version,
            Some("1.0.0".to_string())
        );
        assert_eq!(
            guard_result.updated[0].to_version,
            Some("2.0.0".to_string())
        );
        assert!(!guard_result.install_triggered); // Standard upgrade operation
        assert!(guard_result
            .modified_directories
            .contains(&std::path::PathBuf::from(sps2_config::fixed_paths::LIVE_DIR)));
        assert!(guard_result
            .modified_directories
            .contains(&std::path::PathBuf::from(sps2_config::fixed_paths::BIN_DIR)));
        assert!(guard_result.modified_directories.contains(
            &std::path::PathBuf::from(format!("{}/lib", sps2_config::fixed_paths::LIVE_DIR))
        ));
        assert!(guard_result.modified_directories.contains(
            &std::path::PathBuf::from(format!("{}/share", sps2_config::fixed_paths::LIVE_DIR))
        ));
    }
}

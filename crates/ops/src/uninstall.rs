//! Uninstall command implementation
//!
//! Handles package removal with dependency checking.
//! Delegates to `sps2_install` crate for the actual uninstall logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::{Error, OpsError};
use sps2_events::{
    patterns::UninstallProgressConfig, AppEvent, EventEmitter, GeneralEvent, ProgressManager,
};
use sps2_install::{InstallConfig, Installer, UninstallContext};
use std::time::Instant;
use uuid::Uuid;

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

    let _correlation = ctx.push_correlation_for_packages("uninstall", package_names);

    // Check mode: preview what would be uninstalled
    if ctx.check_mode {
        return preview_uninstall(ctx, package_names).await;
    }

    let progress_manager = ProgressManager::new();
    let uninstall_config = UninstallProgressConfig {
        operation_name: "Uninstalling packages".to_string(),
        package_count: package_names.len() as u64,
    };
    let progress_id = progress_manager.create_uninstall_tracker(uninstall_config);
    let correlation = ctx.current_correlation();
    progress_manager.emit_started(&progress_id, ctx, correlation.as_deref());

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

    progress_manager.complete_operation(&progress_id, ctx);

    Ok(report)
}

/// Preview what would be uninstalled without executing
#[allow(clippy::too_many_lines)]
async fn preview_uninstall(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    use std::collections::HashMap;

    // Get currently installed packages
    let current_packages = ctx.state.get_installed_packages().await?;

    // Find packages to remove
    let mut packages_to_remove = Vec::new();
    let mut not_found_packages = Vec::new();

    for package_name in package_names {
        if let Some(package_id) = current_packages
            .iter()
            .find(|pkg| &pkg.name == package_name)
        {
            packages_to_remove.push(package_id.clone());
        } else {
            not_found_packages.push(package_name.clone());
        }
    }

    // Report packages that would not be found
    for package_name in &not_found_packages {
        ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
            operation: "uninstall".to_string(),
            action: format!("Package {package_name} is not installed"),
            details: HashMap::from([
                ("status".to_string(), "not_installed".to_string()),
                ("action".to_string(), "skip".to_string()),
            ]),
        }));
    }

    let mut preview_removed = Vec::new();
    let mut broken_dependencies = Vec::new();

    // Check each package for dependents
    for package in &packages_to_remove {
        let package_id = sps2_resolver::PackageId::new(package.name.clone(), package.version());

        // Check for dependents
        let dependents = ctx.state.get_package_dependents(&package_id).await?;

        if dependents.is_empty() {
            // Safe to remove
            ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                operation: "uninstall".to_string(),
                action: format!("Would remove {package_id}"),
                details: HashMap::from([
                    ("version".to_string(), package.version().to_string()),
                    ("dependents".to_string(), "0".to_string()),
                    ("status".to_string(), "safe_to_remove".to_string()),
                ]),
            }));
        } else {
            // Has dependents - would break dependencies
            let dependent_names: Vec<String> = dependents
                .iter()
                .map(std::string::ToString::to_string)
                .collect();

            ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                operation: "uninstall".to_string(),
                action: format!("Would remove {package_id} (breaks dependencies)"),
                details: HashMap::from([
                    ("version".to_string(), package.version().to_string()),
                    ("dependents".to_string(), dependents.len().to_string()),
                    ("dependent_packages".to_string(), dependent_names.join(", ")),
                    ("status".to_string(), "breaks_dependencies".to_string()),
                ]),
            }));

            broken_dependencies.extend(dependent_names);
        }

        preview_removed.push(crate::PackageChange {
            name: package.name.clone(),
            from_version: Some(package.version()),
            to_version: None,
            size: None,
        });
    }

    // Show warning for broken dependencies
    if !broken_dependencies.is_empty() {
        ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
            operation: "uninstall".to_string(),
            action: "WARNING: This would break dependencies for:".to_string(),
            details: HashMap::from([
                (
                    "affected_packages".to_string(),
                    broken_dependencies.join(", "),
                ),
                ("severity".to_string(), "error".to_string()),
                (
                    "suggestion".to_string(),
                    "Use --force to override dependency checks".to_string(),
                ),
            ]),
        }));
    }

    // Emit summary
    let total_changes = packages_to_remove.len();
    let mut categories = HashMap::new();
    categories.insert("packages_removed".to_string(), packages_to_remove.len());
    if !broken_dependencies.is_empty() {
        categories.insert("broken_dependencies".to_string(), broken_dependencies.len());
    }
    if !not_found_packages.is_empty() {
        categories.insert("packages_not_found".to_string(), not_found_packages.len());
    }

    ctx.emit(AppEvent::General(GeneralEvent::CheckModeSummary {
        operation: "uninstall".to_string(),
        total_changes,
        categories,
    }));

    // Return preview report (no actual state changes)
    Ok(InstallReport {
        installed: Vec::new(),
        updated: Vec::new(),
        removed: preview_removed,
        state_id: Uuid::nil(), // No state change in preview
        duration_ms: 0,
    })
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
    async fn preview_without_packages_reports_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        let state = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir.clone());
        let (tx, _rx) = sps2_events::channel();
        let config = Config::default();

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

        let preview = preview_uninstall(&ctx, &[]).await.unwrap();
        assert!(preview.removed.is_empty());
    }
}

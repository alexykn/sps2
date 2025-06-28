//! Upgrade command implementation
//!
//! Handles package upgrades, ignoring version constraints to get latest versions.
//! Delegates to `sps2_install` crate for the actual upgrade logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::Error;
use sps2_events::Event;
use sps2_guard::{OperationResult as GuardOperationResult, PackageChange as GuardPackageChange};
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
            std::path::PathBuf::from("/opt/pm/live"),
            std::path::PathBuf::from("/opt/pm/live/bin"),
            std::path::PathBuf::from("/opt/pm/live/lib"),
            std::path::PathBuf::from("/opt/pm/live/share"),
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
        let resolver = Resolver::new(index.clone());
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
        let resolver = Resolver::new(index.clone());
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
            .contains(&std::path::PathBuf::from("/opt/pm/live")));
        assert!(guard_result
            .modified_directories
            .contains(&std::path::PathBuf::from("/opt/pm/live/bin")));
        assert!(guard_result
            .modified_directories
            .contains(&std::path::PathBuf::from("/opt/pm/live/lib")));
        assert!(guard_result
            .modified_directories
            .contains(&std::path::PathBuf::from("/opt/pm/live/share")));
    }
}

//! Uninstall command implementation
//!
//! Handles package removal with dependency checking.
//! Delegates to `sps2_install` crate for the actual uninstall logic.

use crate::{InstallReport, OpsCtx};
use sps2_errors::{Error, OpsError};
use sps2_events::Event;
use sps2_guard::{OperationResult as GuardOperationResult, PackageChange as GuardPackageChange};
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

/// Convert `InstallReport` to `GuardOperationResult` for uninstall operations
fn create_guard_operation_result_for_uninstall(report: &InstallReport) -> GuardOperationResult {
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
        ],
        install_triggered: false, // Uninstall operations never trigger installs
    }
}

/// Uninstall packages with state verification enabled
///
/// This wrapper uses the advanced `GuardedOperation` pattern providing:
/// - Cache warming before operation
/// - Operation-specific verification scoping with orphan detection
/// - Progressive verification when appropriate  
/// - Smart cache invalidation after operation
///
/// # Errors
///
/// Returns an error if:
/// - Pre-uninstall verification fails (when `fail_on_discrepancy` is true)
/// - Uninstallation fails
/// - Post-uninstall verification fails (when `fail_on_discrepancy` is true)
pub async fn uninstall_with_verification(
    ctx: &OpsCtx,
    package_names: &[String],
) -> Result<InstallReport, Error> {
    let package_names_vec = package_names.iter().map(ToString::to_string).collect();

    ctx.guarded_uninstall(package_names_vec)
        .execute(|| async {
            let report = uninstall(ctx, package_names).await?;
            let guard_result = create_guard_operation_result_for_uninstall(&report);
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
    async fn test_uninstall_with_verification_disabled() {
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

        // Test that uninstall_with_verification works without verification enabled
        let result = uninstall_with_verification(&ctx, &[]).await;

        // Should fail with NoPackagesSpecified, not verification error
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("verification"),
                "Should not fail due to verification"
            );
        }
    }

    #[tokio::test]
    async fn test_uninstall_with_verification_enabled() {
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

        // Test that uninstall_with_verification runs verification
        let result = uninstall_with_verification(&ctx, &[]).await;

        // Should still fail with NoPackagesSpecified
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("verification failed"),
                "Should not fail due to verification on empty state"
            );
        }
    }

    #[tokio::test]
    async fn test_create_guard_operation_result_for_uninstall() {
        let report = InstallReport {
            installed: vec![],
            updated: vec![],
            removed: vec![crate::PackageChange {
                name: "test-package".to_string(),
                from_version: Some(sps2_types::Version::parse("1.0.0").unwrap()),
                to_version: None,
                size: Some(1024),
            }],
            state_id: uuid::Uuid::new_v4(),
            duration_ms: 100,
        };

        let guard_result = create_guard_operation_result_for_uninstall(&report);

        assert_eq!(guard_result.installed.len(), 0);
        assert_eq!(guard_result.updated.len(), 0);
        assert_eq!(guard_result.removed.len(), 1);
        assert_eq!(guard_result.removed[0].name, "test-package");
        assert_eq!(
            guard_result.removed[0].from_version,
            Some("1.0.0".to_string())
        );
        assert_eq!(guard_result.removed[0].to_version, None);
        assert!(!guard_result.install_triggered); // Uninstall never triggers install
        assert!(guard_result
            .modified_directories
            .contains(&std::path::PathBuf::from("/opt/pm/live")));
    }
}

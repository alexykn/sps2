//! Small operations implemented in the ops crate
//!
//! This module serves as a public API facade that re-exports operations
//! from specialized modules. All function signatures are preserved for
//! backward compatibility.

// Import all the modularized operations
use crate::health;
use crate::maintenance;
use crate::query;
use crate::repository;
use crate::security;
use crate::self_update as self_update_module;

// Re-export all public functions to maintain API compatibility
pub use health::check_health;
pub use maintenance::{cleanup, history, rollback};
pub use query::{list_packages, package_info, search_packages};
pub use repository::reposync;
pub use security::{audit, update_vulndb, vulndb_stats};
pub use self_update_module::self_update;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpsCtx;
    use sps2_index::Index;
    use tempfile::tempdir;

    async fn create_test_context() -> OpsCtx {
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        // Create necessary directories
        std::fs::create_dir_all(base_path.join("store")).unwrap();
        std::fs::create_dir_all(base_path.join("states")).unwrap();
        std::fs::create_dir_all(base_path.join("live")).unwrap();

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let store = sps2_store::PackageStore::new(base_path.join("store"));

        // Create StateManager with explicit error handling
        let state = match sps2_state::StateManager::new(base_path).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to create StateManager at {base_path:?}: {e}");
                eprintln!("Directory exists: {}", base_path.exists());
                eprintln!(
                    "Directory is writable: {}",
                    base_path
                        .metadata()
                        .map(|m| !m.permissions().readonly())
                        .unwrap_or(false)
                );
                panic!("StateManager creation failed: {e}");
            }
        };
        let mut index = sps2_index::IndexManager::new(base_path);
        let empty_index = Index::new();
        let json = empty_index.to_json().unwrap();
        index.load(Some(&json)).await.unwrap();

        let net = sps2_net::NetClient::with_defaults().unwrap();
        let resolver = sps2_resolver::Resolver::new(index.clone());
        let builder = sps2_builder::Builder::new().with_net(net.clone());

        OpsCtx::new(store, state, index, net, resolver, builder, tx)
    }

    #[tokio::test]
    async fn test_reposync() {
        let ctx = create_test_context().await;
        let result = reposync(&ctx).await.unwrap();
        assert!(result.contains("Repository index"));
    }

    #[tokio::test]
    #[ignore] // Requires /opt/pm SQLite database - fails in CI
    async fn test_list_packages() {
        let ctx = create_test_context().await;

        // In a fresh system, an initial state is automatically created, so list_packages succeeds
        let result = list_packages(&ctx).await;

        // Should succeed and return an empty list (no packages installed yet)
        assert!(result.is_ok());
        let packages = result.unwrap();
        assert!(packages.is_empty());
    }

    #[tokio::test]
    #[ignore] // Requires /opt/pm SQLite database - fails in CI
    async fn test_search_packages() {
        let ctx = create_test_context().await;

        // Search needs an active state to check installed packages
        let result = search_packages(&ctx, "test").await;

        // Should succeed and return an empty list (no packages in index)
        assert!(result.is_ok());
        let results = result.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    #[ignore] // Requires /opt/pm SQLite database - fails in CI
    async fn test_cleanup() {
        let ctx = create_test_context().await;

        // Cleanup also needs an active state
        let result = cleanup(&ctx).await;

        // Should succeed (nothing to clean up in a fresh system)
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore] // Requires /opt/pm SQLite database - fails in CI
    async fn test_history() {
        let ctx = create_test_context().await;

        // History needs an active state to determine which is current
        let result = history(&ctx).await;

        // Should succeed and return a single initial state
        assert!(result.is_ok());
        let states = result.unwrap();
        assert_eq!(states.len(), 1);
    }

    #[tokio::test]
    async fn test_check_health() {
        let ctx = create_test_context().await;
        let health = check_health(&ctx).await.unwrap();

        // Should have checks for store, state, and index
        assert!(health.components.contains_key("store"));
        assert!(health.components.contains_key("state"));
        assert!(health.components.contains_key("index"));
    }

    #[tokio::test]
    #[ignore] // Requires /opt/pm SQLite database - fails in CI
    async fn test_audit() {
        let ctx = create_test_context().await;

        // Audit needs an active state to check installed packages
        let result = audit(&ctx, None, false, sps2_audit::Severity::Low).await;

        // Should succeed and return an audit report (no packages to audit)
        assert!(result.is_ok());
        let audit_report = result.unwrap();
        assert_eq!(audit_report.summary.packages_scanned, 0);
    }
}

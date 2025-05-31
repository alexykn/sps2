//! Integration tests for install crate

#[cfg(test)]
mod tests {
    use sps2_index::{DependencyInfo, Index, IndexManager, VersionEntry};
    use sps2_install::*;
    use sps2_resolver::Resolver;
    use sps2_state::StateManager;
    use sps2_store::PackageStore;
    use sps2_types::{PackageSpec, StateInfo, Version};
    use tempfile::tempdir;
    use uuid::Uuid;

    async fn create_test_setup() -> (Installer, tempfile::TempDir) {
        let temp = tempdir().unwrap();

        // Create index with test packages
        let mut index = Index::new();

        // Add curl package
        let curl_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "curl_hash".to_string(),
            download_url: "https://example.com/curl-8.5.0.sp".to_string(),
            minisig_url: "https://example.com/curl-8.5.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["openssl>=3.0.0".to_string()],
                build: vec![],
            },
            sbom: None,
            description: Some("HTTP client".to_string()),
            homepage: None,
            license: None,
        };

        // Add openssl package
        let openssl_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "openssl_hash".to_string(),
            download_url: "https://example.com/openssl-3.0.0.sp".to_string(),
            minisig_url: "https://example.com/openssl-3.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo::default(),
            sbom: None,
            description: Some("Crypto library".to_string()),
            homepage: None,
            license: None,
        };

        index.add_version("curl".to_string(), "8.5.0".to_string(), curl_entry);
        index.add_version("openssl".to_string(), "3.0.0".to_string(), openssl_entry);

        // Setup components
        let mut index_manager = IndexManager::new(temp.path());
        let json = index.to_json().unwrap();
        index_manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(index_manager);
        let state_manager = StateManager::new(temp.path()).await.unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());
        let config = InstallConfig::default().with_concurrency(2);

        let installer = Installer::new(config, resolver, state_manager, store);

        (installer, temp)
    }

    #[tokio::test]
    async fn test_installer_creation() {
        let (_installer, _temp) = create_test_setup().await;
        // Installer was created successfully - config is private
    }

    #[tokio::test]
    async fn test_install_context_creation() {
        let context = InstallContext::new()
            .add_package(PackageSpec::parse("curl>=8.0.0").unwrap())
            .add_package(PackageSpec::parse("wget>=1.21.0").unwrap())
            .with_force(true)
            .with_dry_run(false);

        assert_eq!(context.packages.len(), 2);
        assert!(context.force);
        assert!(!context.dry_run);

        let package_names: Vec<&str> = context
            .packages
            .iter()
            .map(|spec| spec.name.as_str())
            .collect();
        assert!(package_names.contains(&"curl"));
        assert!(package_names.contains(&"wget"));
    }

    #[tokio::test]
    async fn test_uninstall_context_creation() {
        let context = UninstallContext::new()
            .add_package("curl".to_string())
            .add_package("wget".to_string())
            .with_autoremove(true)
            .with_force(false);

        assert_eq!(context.packages.len(), 2);
        assert!(context.autoremove);
        assert!(!context.force);
        assert!(context.packages.contains(&"curl".to_string()));
        assert!(context.packages.contains(&"wget".to_string()));
    }

    #[tokio::test]
    async fn test_update_context_creation() {
        let context = UpdateContext::new()
            .add_package("curl".to_string())
            .with_upgrade(true)
            .with_dry_run(true);

        assert_eq!(context.packages.len(), 1);
        assert!(context.upgrade);
        assert!(context.dry_run);
        assert_eq!(context.packages[0], "curl");
    }

    #[tokio::test]
    async fn test_install_result() {
        let state_id = Uuid::new_v4();
        let mut result = InstallResult::new(state_id);

        assert_eq!(result.state_id, state_id);
        assert_eq!(result.total_changes(), 0);

        let package_id =
            sps2_resolver::PackageId::new("curl".to_string(), Version::parse("8.5.0").unwrap());

        result.add_installed(package_id.clone());
        assert_eq!(result.total_changes(), 1);
        assert_eq!(result.installed_packages.len(), 1);

        result.add_updated(package_id.clone());
        assert_eq!(result.total_changes(), 2);
        assert_eq!(result.updated_packages.len(), 1);

        result.add_removed(package_id);
        assert_eq!(result.total_changes(), 3);
        assert_eq!(result.removed_packages.len(), 1);
    }

    #[tokio::test]
    async fn test_state_info() {
        let state_id = Uuid::new_v4();
        let parent_id = Uuid::new_v4();
        let timestamp = chrono::Utc::now() - chrono::Duration::hours(2);

        let state_info = StateInfo {
            id: state_id,
            timestamp,
            parent: Some(parent_id),
            operation: "install".to_string(),
            package_count: 5,
            total_size: 1024 * 1024 * 100, // 100 MB
        };

        // Test state has parent
        assert!(state_info.parent.is_some());
        assert_eq!(state_info.package_count, 5);
        assert_eq!(state_info.operation, "install");

        // Test root state
        let root_state = StateInfo {
            id: state_id,
            timestamp,
            parent: None,
            operation: "initial".to_string(),
            package_count: 0,
            total_size: 0,
        };

        assert!(root_state.parent.is_none());
        assert_eq!(root_state.package_count, 0);
    }

    #[tokio::test]
    async fn test_install_config_customization() {
        let config = InstallConfig::default();
        assert_eq!(config.max_concurrency, 4);
        assert_eq!(config.download_timeout, 300);
        assert_eq!(config.state_retention, 10);

        let custom_config = InstallConfig::default()
            .with_concurrency(8)
            .with_timeout(600)
            .with_apfs(false)
            .with_retention(20);

        assert_eq!(custom_config.max_concurrency, 8);
        assert_eq!(custom_config.download_timeout, 600);
        assert!(!custom_config.enable_apfs);
        assert_eq!(custom_config.state_retention, 20);
    }

    #[tokio::test]
    async fn test_atomic_installer_creation() {
        let temp = tempdir().unwrap();
        let state_manager = StateManager::new(temp.path()).await.unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        let atomic_installer = AtomicInstaller::new(state_manager, store).await.unwrap();

        // Just verify creation succeeds - internal fields are private
        drop(atomic_installer);
    }

    #[tokio::test]
    async fn test_parallel_executor_creation() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        let executor = ParallelExecutor::new(store)
            .unwrap()
            .with_concurrency(8)
            .with_timeout(std::time::Duration::from_secs(600));

        // Executor created with custom settings - fields are private
        let _ = executor;
    }

    #[tokio::test]
    async fn test_execution_context() {
        let context = ExecutionContext::new();
        // Context created successfully - event_sender is private
        let _ = context;
    }

    // Note: Full end-to-end integration tests would require:
    // 1. Real package files in store
    // 2. Network access for downloads
    // 3. Proper filesystem permissions for /opt/pm
    // 4. APFS filesystem for optimal testing
    //
    // These tests focus on the API and component integration
}

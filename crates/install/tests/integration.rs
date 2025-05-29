//! Integration tests for install crate

#[cfg(test)]
mod tests {
    use spsv2_index::{DependencyInfo, Index, IndexManager, VersionEntry};
    use spsv2_install::*;
    use spsv2_resolver::Resolver;
    use spsv2_state::StateManager;
    use spsv2_store::PackageStore;
    use spsv2_types::{PackageSpec, Version};
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
            sha256: "curl_hash".to_string(),
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
            sha256: "openssl_hash".to_string(),
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
        let store = PackageStore::new(temp.path()).await.unwrap();
        let config = InstallConfig::default().with_concurrency(2);

        let installer = Installer::new(config, resolver, state_manager, store);

        (installer, temp)
    }

    #[tokio::test]
    async fn test_installer_creation() {
        let (installer, _temp) = create_test_setup().await;
        assert_eq!(installer.config.max_concurrency, 2);
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
            spsv2_resolver::PackageId::new("curl".to_string(), Version::parse("8.5.0").unwrap());

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

        let packages = vec![
            spsv2_resolver::PackageId::new("curl".to_string(), Version::parse("8.5.0").unwrap()),
            spsv2_resolver::PackageId::new("wget".to_string(), Version::parse("1.21.3").unwrap()),
            spsv2_resolver::PackageId::new("jq".to_string(), Version::parse("1.7.0").unwrap()),
            spsv2_resolver::PackageId::new("git".to_string(), Version::parse("2.41.0").unwrap()),
            spsv2_resolver::PackageId::new("vim".to_string(), Version::parse("9.0.0").unwrap()),
        ];

        let state_info = StateInfo {
            id: state_id,
            timestamp,
            parent_id: Some(parent_id),
            package_count: packages.len(),
            packages: packages.clone(),
        };

        assert!(!state_info.is_root());
        assert_eq!(state_info.package_count, 5);

        // Test age calculation
        let age = state_info.age();
        assert!(age.num_hours() >= 1);

        // Test package summary
        let summary = state_info.package_summary();
        assert!(summary.contains("curl-8.5.0"));
        assert!(summary.contains("and 2 more"));

        // Test with fewer packages
        let small_state = StateInfo {
            id: state_id,
            timestamp,
            parent_id: Some(parent_id),
            package_count: 2,
            packages: packages.into_iter().take(2).collect(),
        };

        let small_summary = small_state.package_summary();
        assert!(small_summary.contains("curl-8.5.0"));
        assert!(small_summary.contains("wget-1.21.3"));
        assert!(!small_summary.contains("more"));

        // Test root state
        let root_state = StateInfo {
            id: state_id,
            timestamp,
            parent_id: None,
            package_count: 0,
            packages: vec![],
        };

        assert!(root_state.is_root());
        assert_eq!(root_state.package_summary(), "No packages");
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
        let store = PackageStore::new(temp.path()).await.unwrap();

        let atomic_installer = AtomicInstaller::new(state_manager, store);

        // Just verify creation succeeds
        assert_eq!(
            atomic_installer.live_path,
            std::path::PathBuf::from("/opt/pm/live")
        );
    }

    #[tokio::test]
    async fn test_parallel_executor_creation() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path()).await.unwrap();

        let executor = ParallelExecutor::new(store)
            .with_concurrency(8)
            .with_timeout(std::time::Duration::from_secs(600));

        assert_eq!(executor.max_concurrency, 8);
        assert_eq!(
            executor.download_timeout,
            std::time::Duration::from_secs(600)
        );
    }

    #[tokio::test]
    async fn test_execution_context() {
        let context = ExecutionContext::new();
        assert!(context.event_sender.is_none());

        // Test event sending (should not panic)
        context.send_event(spsv2_events::Event::PackageInstalling {
            name: "test".to_string(),
            version: Version::parse("1.0.0").unwrap(),
        });
    }

    // Note: Full end-to-end integration tests would require:
    // 1. Real package files in store
    // 2. Network access for downloads
    // 3. Proper filesystem permissions for /opt/pm
    // 4. APFS filesystem for optimal testing
    //
    // These tests focus on the API and component integration
}

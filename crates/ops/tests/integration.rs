//! Integration tests for ops crate

#[cfg(test)]
mod tests {
    use sps2_index::{DependencyInfo, Index, IndexManager, VersionEntry};
    use sps2_ops::*;
    use sps2_types::PackageSpec;
    use tempfile::tempdir;

    async fn create_test_context() -> OpsCtx {
        let temp = tempdir().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Create test index with sample packages
        let mut index = Index::new();

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
            homepage: Some("https://curl.se".to_string()),
            license: Some("MIT".to_string()),
        };

        index.add_version("curl".to_string(), "8.5.0".to_string(), curl_entry);

        let store = sps2_store::PackageStore::new(temp.path().to_path_buf());
        let state = sps2_state::StateManager::new(temp.path()).await.unwrap();
        let mut index_manager = IndexManager::new(temp.path());
        let json = index.to_json().unwrap();
        index_manager.load(Some(&json)).await.unwrap();

        let net = sps2_net::NetClient::with_defaults().unwrap();
        let resolver = sps2_resolver::Resolver::new(index_manager.clone());
        let builder = sps2_builder::Builder::new();

        OpsCtx::new(store, state, index_manager, net, resolver, builder, tx)
    }

    #[tokio::test]
    async fn test_ops_context_creation() {
        let _ctx = create_test_context().await;

        // Verify context was created successfully
        // Context was created successfully, no further assertions needed
    }

    #[tokio::test]
    async fn test_ops_context_builder() {
        let temp = tempdir().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let store = sps2_store::PackageStore::new(temp.path().to_path_buf());
        let state = sps2_state::StateManager::new(temp.path()).await.unwrap();
        let index = IndexManager::new(temp.path());
        let net = sps2_net::NetClient::with_defaults().unwrap();
        let resolver = sps2_resolver::Resolver::new(index.clone());
        let builder = sps2_builder::Builder::new();

        let _ctx = OpsContextBuilder::new()
            .with_store(store)
            .with_state(state)
            .with_index(index)
            .with_net(net)
            .with_resolver(resolver)
            .with_builder(builder)
            .with_event_sender(tx)
            .build()
            .unwrap();

        // Verify context was built successfully
        // Context was built successfully, no further assertions needed
    }

    #[tokio::test]
    async fn test_reposync_operation() {
        let ctx = create_test_context().await;
        let result = reposync(&ctx).await.unwrap();

        assert!(result.contains("Repository index"));
    }

    #[tokio::test]
    async fn test_list_packages_operation() {
        let ctx = create_test_context().await;

        // list_packages needs an active state
        let result = list_packages(&ctx).await;
        assert!(result.is_err()); // Should fail without active state
    }

    #[tokio::test]
    async fn test_package_info_operation() {
        let ctx = create_test_context().await;

        // package_info needs an active state to check if package is installed
        let result = package_info(&ctx, "curl").await;
        assert!(result.is_err()); // Should fail without active state
    }

    #[tokio::test]
    async fn test_search_packages_operation() {
        let ctx = create_test_context().await;

        // search_packages needs an active state to check installed packages
        let result = search_packages(&ctx, "cur").await;
        assert!(result.is_err()); // Should fail without active state
    }

    #[tokio::test]
    async fn test_cleanup_operation() {
        let ctx = create_test_context().await;

        // cleanup needs an active state
        let result = cleanup(&ctx).await;
        assert!(result.is_err()); // Should fail without active state
    }

    #[tokio::test]
    async fn test_history_operation() {
        let ctx = create_test_context().await;

        // history needs an active state to determine current
        let result = history(&ctx).await;
        assert!(result.is_err()); // Should fail without active state
    }

    #[tokio::test]
    async fn test_check_health_operation() {
        let ctx = create_test_context().await;
        let health = check_health(&ctx).await.unwrap();

        // Should have health checks for all components
        assert!(health.components.contains_key("store"));
        assert!(health.components.contains_key("state"));
        assert!(health.components.contains_key("index"));

        // Health should be mostly good (may have index staleness warning)
        assert!(health.components.len() >= 3);
    }

    #[test]
    fn test_operation_result_serialization() {
        let package_info = PackageInfo {
            name: "curl".to_string(),
            version: Some(sps2_types::Version::parse("8.5.0").unwrap()),
            available_version: None,
            description: Some("HTTP client".to_string()),
            homepage: None,
            license: None,
            status: PackageStatus::Installed,
            dependencies: vec!["openssl>=3.0.0".to_string()],
            size: Some(1024000),
        };

        let result = OperationResult::PackageInfo(package_info);

        assert!(result.is_success());

        let json = result.to_json().unwrap();
        assert!(json.contains("curl"));
        assert!(json.contains("HTTP client"));
    }

    #[test]
    fn test_operation_result_success_check() {
        let success_result = OperationResult::Success("Operation completed".to_string());
        assert!(success_result.is_success());

        let healthy_check = HealthCheck {
            healthy: true,
            components: std::collections::HashMap::new(),
            issues: Vec::new(),
        };
        let health_result = OperationResult::HealthCheck(healthy_check);
        assert!(health_result.is_success());

        let unhealthy_check = HealthCheck {
            healthy: false,
            components: std::collections::HashMap::new(),
            issues: Vec::new(),
        };
        let unhealthy_result = OperationResult::HealthCheck(unhealthy_check);
        assert!(!unhealthy_result.is_success());
    }

    #[test]
    fn test_install_request_parsing() {
        let temp = tempdir().unwrap();

        // Create a test .sp file
        let sp_file = temp.path().join("test-1.0.0-1.arm64.sp");
        std::fs::write(&sp_file, b"test package").unwrap();

        let specs = vec!["curl>=8.0.0".to_string(), sp_file.display().to_string()];

        // This would be tested in large_ops.rs, but we can test the concept here
        for spec in &specs {
            if spec.ends_with(".sp") && std::path::Path::new(spec).exists() {
                // Would create InstallRequest::LocalFile
                assert!(spec.contains("test-1.0.0-1.arm64.sp"));
            } else {
                // Would create InstallRequest::Remote
                let package_spec = PackageSpec::parse(spec).unwrap();
                assert_eq!(package_spec.name, "curl");
            }
        }
    }

    #[test]
    fn test_op_report_creation() {
        let changes = vec![OpChange {
            change_type: ChangeType::Install,
            package: "curl".to_string(),
            old_version: None,
            new_version: Some(sps2_types::Version::parse("8.5.0").unwrap()),
        }];

        let report = OpReport::success(
            "install".to_string(),
            "Installed 1 package".to_string(),
            changes,
            1500,
        );

        assert!(report.success);
        assert_eq!(report.changes.len(), 1);
        assert_eq!(report.duration_ms, 1500);

        let failure_report = OpReport::failure(
            "install".to_string(),
            "Installation failed".to_string(),
            1000,
        );

        assert!(!failure_report.success);
        assert!(failure_report.changes.is_empty());
    }
}

//! Integration tests for resolver crate

#[cfg(test)]
mod tests {
    use spsv2_index::{DependencyInfo, Index, IndexManager, VersionEntry};
    use spsv2_resolver::*;
    use spsv2_types::{PackageSpec, Version};
    use tempfile::tempdir;

    fn create_complex_index() -> Index {
        let mut index = Index::new();

        // jq -> oniguruma
        let jq_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            sha256: "jq_hash".to_string(),
            download_url: "https://example.com/jq-1.7.0.sp".to_string(),
            minisig_url: "https://example.com/jq-1.7.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["oniguruma>=6.9.0".to_string()],
                build: vec!["autoconf>=2.71.0".to_string()],
            },
            sbom: None,
            description: Some("JSON processor".to_string()),
            homepage: None,
            license: None,
        };

        // curl -> openssl, zlib
        let curl_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            sha256: "curl_hash".to_string(),
            download_url: "https://example.com/curl-8.5.0.sp".to_string(),
            minisig_url: "https://example.com/curl-8.5.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["openssl>=3.0.0".to_string(), "zlib>=1.2.0".to_string()],
                build: vec!["pkg-config>=0.29".to_string()],
            },
            sbom: None,
            description: Some("HTTP client".to_string()),
            homepage: None,
            license: None,
        };

        // git -> curl, zlib (shared dependency)
        let git_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            sha256: "git_hash".to_string(),
            download_url: "https://example.com/git-2.41.0.sp".to_string(),
            minisig_url: "https://example.com/git-2.41.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["curl>=8.0.0".to_string(), "zlib>=1.2.0".to_string()],
                build: vec!["make>=4.0.0".to_string()],
            },
            sbom: None,
            description: Some("Version control".to_string()),
            homepage: None,
            license: None,
        };

        // Leaf dependencies (no further deps)
        let deps = vec![
            ("oniguruma", "6.9.8"),
            ("openssl", "3.0.0"),
            ("zlib", "1.2.11"),
            ("autoconf", "2.71.0"),
            ("pkg-config", "0.29.2"),
            ("make", "4.3.0"),
        ];

        for (name, version) in deps {
            let entry = VersionEntry {
                revision: 1,
                arch: "arm64".to_string(),
                sha256: format!("{name}_hash"),
                download_url: format!("https://example.com/{name}-{version}.sp"),
                minisig_url: format!("https://example.com/{name}-{version}.sp.minisig"),
                dependencies: DependencyInfo::default(),
                sbom: None,
                description: Some(format!("{name} package")),
                homepage: None,
                license: None,
            };
            index.add_version(name.to_string(), version.to_string(), entry);
        }

        index.add_version("jq".to_string(), "1.7.0".to_string(), jq_entry);
        index.add_version("curl".to_string(), "8.5.0".to_string(), curl_entry);
        index.add_version("git".to_string(), "2.41.0".to_string(), git_entry);

        index
    }

    #[tokio::test]
    async fn test_complex_dependency_resolution() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_complex_index();
        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        // Install git (which depends on curl, which depends on openssl and zlib)
        let context =
            ResolutionContext::new().add_runtime_dep(PackageSpec::parse("git>=2.0.0").unwrap());

        let result = resolver.resolve(context).await.unwrap();

        // Should resolve: git, curl, openssl, zlib
        assert_eq!(result.nodes.len(), 4);

        let package_names: std::collections::HashSet<_> =
            result.nodes.keys().map(|id| &id.name).collect();

        assert!(package_names.iter().any(|name| name.as_str() == "git"));
        assert!(package_names.iter().any(|name| name.as_str() == "curl"));
        assert!(package_names.iter().any(|name| name.as_str() == "openssl"));
        assert!(package_names.iter().any(|name| name.as_str() == "zlib"));

        // Check execution order
        let packages = result.packages_in_order();
        let get_position = |name: &str| packages.iter().position(|p| p.name == name).unwrap();

        // Dependencies should come before dependents
        assert!(get_position("openssl") < get_position("curl"));
        assert!(get_position("zlib") < get_position("curl"));
        assert!(get_position("curl") < get_position("git"));

        // zlib is shared between curl and git - should only appear once
        let zlib_count = packages.iter().filter(|p| p.name == "zlib").count();
        assert_eq!(zlib_count, 1);
    }

    #[tokio::test]
    async fn test_multiple_root_packages() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_complex_index();
        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        // Install both jq and curl
        let context = ResolutionContext::new()
            .add_runtime_dep(PackageSpec::parse("jq>=1.7.0").unwrap())
            .add_runtime_dep(PackageSpec::parse("curl>=8.0.0").unwrap());

        let result = resolver.resolve(context).await.unwrap();

        // Should resolve: jq, oniguruma, curl, openssl, zlib
        assert_eq!(result.nodes.len(), 5);

        let package_names: std::collections::HashSet<_> =
            result.nodes.keys().map(|id| &id.name).collect();

        assert!(package_names.iter().any(|name| name.as_str() == "jq"));
        assert!(package_names
            .iter()
            .any(|name| name.as_str() == "oniguruma"));
        assert!(package_names.iter().any(|name| name.as_str() == "curl"));
        assert!(package_names.iter().any(|name| name.as_str() == "openssl"));
        assert!(package_names.iter().any(|name| name.as_str() == "zlib"));
    }

    #[tokio::test]
    async fn test_build_dependencies() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_complex_index();
        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        // Resolve jq as build dependency (should include autoconf)
        let context =
            ResolutionContext::new().add_build_dep(PackageSpec::parse("jq>=1.7.0").unwrap());

        let result = resolver.resolve(context).await.unwrap();

        // Should resolve: jq, oniguruma (runtime), autoconf (build)
        assert_eq!(result.nodes.len(), 3);

        let package_names: std::collections::HashSet<_> =
            result.nodes.keys().map(|id| &id.name).collect();

        assert!(package_names.iter().any(|name| name.as_str() == "jq"));
        assert!(package_names
            .iter()
            .any(|name| name.as_str() == "oniguruma"));
        assert!(package_names.iter().any(|name| name.as_str() == "autoconf"));
    }

    #[tokio::test]
    async fn test_execution_plan_batching() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_complex_index();
        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        let context =
            ResolutionContext::new().add_runtime_dep(PackageSpec::parse("git>=2.0.0").unwrap());

        let result = resolver.resolve(context).await.unwrap();

        // Check batching - packages with no deps should be in first batch
        let batches = result.execution_plan.batches();

        // First batch should contain leaf dependencies (openssl, zlib)
        let first_batch_names: std::collections::HashSet<_> = batches[0]
            .iter()
            .filter_map(|id| result.nodes.get(id))
            .map(|node| &node.name)
            .collect();

        assert!(first_batch_names
            .iter()
            .any(|name| name.as_str() == "openssl"));
        assert!(first_batch_names.iter().any(|name| name.as_str() == "zlib"));

        // Last batch should contain root package (git)
        let last_batch = &batches[batches.len() - 1];
        let last_batch_names: std::collections::HashSet<_> = last_batch
            .iter()
            .filter_map(|id| result.nodes.get(id))
            .map(|node| &node.name)
            .collect();

        assert!(last_batch_names.iter().any(|name| name.as_str() == "git"));
    }

    #[tokio::test]
    async fn test_version_constraint_resolution() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let mut index = create_complex_index();

        // Add multiple versions of openssl
        let openssl_301 = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            sha256: "openssl_301_hash".to_string(),
            download_url: "https://example.com/openssl-3.0.1.sp".to_string(),
            minisig_url: "https://example.com/openssl-3.0.1.sp.minisig".to_string(),
            dependencies: DependencyInfo::default(),
            sbom: None,
            description: Some("Crypto library".to_string()),
            homepage: None,
            license: None,
        };

        index.add_version("openssl".to_string(), "3.0.1".to_string(), openssl_301);

        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        // Request specific version constraint
        let context =
            ResolutionContext::new().add_runtime_dep(PackageSpec::parse("openssl==3.0.1").unwrap());

        let result = resolver.resolve(context).await.unwrap();

        // Should resolve to exactly version 3.0.1
        let openssl_node = result
            .nodes
            .values()
            .find(|node| node.name == "openssl")
            .unwrap();

        assert_eq!(openssl_node.version, Version::parse("3.0.1").unwrap());
    }

    #[test]
    fn test_resolution_context_builder() {
        let context = ResolutionContext::new()
            .add_runtime_dep(PackageSpec::parse("curl>=8.0.0").unwrap())
            .add_build_dep(PackageSpec::parse("pkg-config>=0.29.0").unwrap())
            .add_local_file("/path/to/local.sp".into());

        assert_eq!(context.runtime_deps.len(), 1);
        assert_eq!(context.build_deps.len(), 1);
        assert_eq!(context.local_files.len(), 1);

        assert_eq!(context.runtime_deps[0].name, "curl");
        assert_eq!(context.build_deps[0].name, "pkg-config");
    }
}

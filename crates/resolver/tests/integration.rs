//! Integration tests for resolver crate

#[cfg(test)]
mod tests {
    use sps2_index::{DependencyInfo, Index, IndexManager, VersionEntry};
    use sps2_resolver::*;
    use sps2_types::{PackageSpec, Version};
    use tempfile::tempdir;

    fn create_complex_index() -> Index {
        let mut index = Index::new();

        // jq -> oniguruma
        let jq_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "jq_hash".to_string(),
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
            blake3: "curl_hash".to_string(),
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
            blake3: "git_hash".to_string(),
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
                blake3: format!("{name}_hash"),
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
            blake3: "openssl_301_hash".to_string(),
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

    // SAT solver integration tests

    #[tokio::test]
    async fn test_sat_basic_resolution() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_complex_index();
        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        // Resolve using SAT solver
        let context =
            ResolutionContext::new().add_runtime_dep(PackageSpec::parse("curl>=8.0.0").unwrap());

        let result = resolver.resolve_with_sat(context).await.unwrap();

        // Should resolve: curl, openssl, zlib
        assert_eq!(result.nodes.len(), 3);

        let package_names: std::collections::HashSet<_> =
            result.nodes.keys().map(|id| &id.name).collect();

        assert!(package_names.iter().any(|&name| name == "curl"));
        assert!(package_names.iter().any(|&name| name == "openssl"));
        assert!(package_names.iter().any(|&name| name == "zlib"));
    }

    #[tokio::test]
    async fn test_sat_conflict_detection() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let mut index = Index::new();

        // Create conflicting package versions
        // foo 1.0.0 depends on bar==1.0.0
        let foo1 = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "foo1_hash".to_string(),
            download_url: "https://example.com/foo-1.0.0.sp".to_string(),
            minisig_url: "https://example.com/foo-1.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["bar==1.0.0".to_string()],
                build: vec![],
            },
            sbom: None,
            description: Some("Foo package".to_string()),
            homepage: None,
            license: None,
        };

        // baz 1.0.0 depends on bar==2.0.0
        let baz1 = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "baz1_hash".to_string(),
            download_url: "https://example.com/baz-1.0.0.sp".to_string(),
            minisig_url: "https://example.com/baz-1.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["bar==2.0.0".to_string()],
                build: vec![],
            },
            sbom: None,
            description: Some("Baz package".to_string()),
            homepage: None,
            license: None,
        };

        // bar versions
        let bar1 = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "bar1_hash".to_string(),
            download_url: "https://example.com/bar-1.0.0.sp".to_string(),
            minisig_url: "https://example.com/bar-1.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo::default(),
            sbom: None,
            description: Some("Bar v1".to_string()),
            homepage: None,
            license: None,
        };

        let bar2 = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "bar2_hash".to_string(),
            download_url: "https://example.com/bar-2.0.0.sp".to_string(),
            minisig_url: "https://example.com/bar-2.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo::default(),
            sbom: None,
            description: Some("Bar v2".to_string()),
            homepage: None,
            license: None,
        };

        index.add_version("foo".to_string(), "1.0.0".to_string(), foo1);
        index.add_version("baz".to_string(), "1.0.0".to_string(), baz1);
        index.add_version("bar".to_string(), "1.0.0".to_string(), bar1);
        index.add_version("bar".to_string(), "2.0.0".to_string(), bar2);

        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        // Try to install both foo and baz - should conflict
        let context = ResolutionContext::new()
            .add_runtime_dep(PackageSpec::parse("foo==1.0.0").unwrap())
            .add_runtime_dep(PackageSpec::parse("baz==1.0.0").unwrap());

        let result = resolver.resolve_with_sat(context).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("conflict") || err_msg.contains("Conflict"));
    }

    #[tokio::test]
    async fn test_sat_version_preference() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let mut index = Index::new();

        // Create multiple versions
        for version in ["1.0.0", "1.1.0", "1.2.0", "2.0.0"] {
            let entry = VersionEntry {
                revision: 1,
                arch: "arm64".to_string(),
                blake3: format!("test_{version}_hash"),
                download_url: format!("https://example.com/test-{version}.sp"),
                minisig_url: format!("https://example.com/test-{version}.sp.minisig"),
                dependencies: DependencyInfo::default(),
                sbom: None,
                description: Some(format!("Test v{version}")),
                homepage: None,
                license: None,
            };
            index.add_version("test".to_string(), version.to_string(), entry);
        }

        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        // Test compatible release constraint
        // ~=1.1.0 means >=1.1.0 but same major.minor, so it matches 1.1.0 but not 1.2.0
        let context =
            ResolutionContext::new().add_runtime_dep(PackageSpec::parse("test~=1.1.0").unwrap());

        let result = resolver.resolve_with_sat(context).await.unwrap();

        assert_eq!(result.nodes.len(), 1);
        let test_node = result.nodes.values().next().unwrap();

        // Should select 1.1.0 (only version matching ~=1.1.0)
        assert_eq!(test_node.version, Version::parse("1.1.0").unwrap());

        // Now test that version preference works with a looser constraint
        let context2 = ResolutionContext::new()
            .add_runtime_dep(PackageSpec::parse("test>=1.0.0,<2.0.0").unwrap());

        let result2 = resolver.resolve_with_sat(context2).await.unwrap();

        assert_eq!(result2.nodes.len(), 1);
        let test_node2 = result2.nodes.values().next().unwrap();

        // Should select 1.2.0 (highest version matching >=1.0.0,<2.0.0)
        assert_eq!(test_node2.version, Version::parse("1.2.0").unwrap());
    }

    #[tokio::test]
    async fn test_sat_complex_constraints() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let mut index = Index::new();

        // Package A depends on B with complex constraints
        let a_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "a_hash".to_string(),
            download_url: "https://example.com/a-1.0.0.sp".to_string(),
            minisig_url: "https://example.com/a-1.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["b>=1.2.0,<2.0.0,!=1.5.0".to_string()],
                build: vec![],
            },
            sbom: None,
            description: Some("Package A".to_string()),
            homepage: None,
            license: None,
        };

        // Create B versions
        for version in ["1.0.0", "1.2.0", "1.5.0", "1.8.0", "2.0.0"] {
            let entry = VersionEntry {
                revision: 1,
                arch: "arm64".to_string(),
                blake3: format!("b_{version}_hash"),
                download_url: format!("https://example.com/b-{version}.sp"),
                minisig_url: format!("https://example.com/b-{version}.sp.minisig"),
                dependencies: DependencyInfo::default(),
                sbom: None,
                description: Some(format!("Package B v{version}")),
                homepage: None,
                license: None,
            };
            index.add_version("b".to_string(), version.to_string(), entry);
        }

        index.add_version("a".to_string(), "1.0.0".to_string(), a_entry);

        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        let context =
            ResolutionContext::new().add_runtime_dep(PackageSpec::parse("a==1.0.0").unwrap());

        let result = resolver.resolve_with_sat(context).await.unwrap();

        assert_eq!(result.nodes.len(), 2);

        // Find B in the result
        let b_node = result
            .nodes
            .values()
            .find(|n| n.name == "b")
            .expect("B should be resolved");

        // Should select 1.8.0 (highest version matching constraints, excluding 1.5.0)
        assert_eq!(b_node.version, Version::parse("1.8.0").unwrap());
    }

    #[tokio::test]
    async fn test_sat_transitive_dependencies() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let mut index = Index::new();

        // Create deep dependency chain: A -> B -> C -> D
        let a_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "a_hash".to_string(),
            download_url: "https://example.com/a-1.0.0.sp".to_string(),
            minisig_url: "https://example.com/a-1.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["b>=1.0.0".to_string()],
                build: vec![],
            },
            sbom: None,
            description: Some("Package A".to_string()),
            homepage: None,
            license: None,
        };

        let b_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "b_hash".to_string(),
            download_url: "https://example.com/b-1.0.0.sp".to_string(),
            minisig_url: "https://example.com/b-1.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["c>=1.0.0".to_string()],
                build: vec![],
            },
            sbom: None,
            description: Some("Package B".to_string()),
            homepage: None,
            license: None,
        };

        let c_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "c_hash".to_string(),
            download_url: "https://example.com/c-1.0.0.sp".to_string(),
            minisig_url: "https://example.com/c-1.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["d>=1.0.0".to_string()],
                build: vec![],
            },
            sbom: None,
            description: Some("Package C".to_string()),
            homepage: None,
            license: None,
        };

        let d_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "d_hash".to_string(),
            download_url: "https://example.com/d-1.0.0.sp".to_string(),
            minisig_url: "https://example.com/d-1.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo::default(),
            sbom: None,
            description: Some("Package D".to_string()),
            homepage: None,
            license: None,
        };

        index.add_version("a".to_string(), "1.0.0".to_string(), a_entry);
        index.add_version("b".to_string(), "1.0.0".to_string(), b_entry);
        index.add_version("c".to_string(), "1.0.0".to_string(), c_entry);
        index.add_version("d".to_string(), "1.0.0".to_string(), d_entry);

        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        let context =
            ResolutionContext::new().add_runtime_dep(PackageSpec::parse("a>=1.0.0").unwrap());

        let result = resolver.resolve_with_sat(context).await.unwrap();

        // Should resolve all 4 packages
        assert_eq!(result.nodes.len(), 4);

        let package_names: std::collections::HashSet<_> =
            result.nodes.keys().map(|id| &id.name).collect();

        assert!(package_names.iter().any(|&name| name == "a"));
        assert!(package_names.iter().any(|&name| name == "b"));
        assert!(package_names.iter().any(|&name| name == "c"));
        assert!(package_names.iter().any(|&name| name == "d"));

        // Check execution order
        let packages = result.packages_in_order();
        let get_position = |name: &str| packages.iter().position(|p| p.name == name).unwrap();

        // Dependencies should come before dependents
        assert!(get_position("d") < get_position("c"));
        assert!(get_position("c") < get_position("b"));
        assert!(get_position("b") < get_position("a"));
    }
}

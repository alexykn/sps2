//! Integration tests for index crate

#[cfg(test)]
mod tests {
    use spsv2_index::*;
    use spsv2_types::PackageSpec;
    use tempfile::tempdir;

    fn create_test_index() -> Index {
        let mut index = Index::new();

        // Add jq package
        let jq_170 = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            sha256: "abc123".to_string(),
            download_url: "https://cdn.example.com/jq-1.7.0-1.arm64.sp".to_string(),
            minisig_url: "https://cdn.example.com/jq-1.7.0-1.arm64.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["oniguruma==6.9.8".to_string()],
                build: vec!["autoconf>=2.71".to_string()],
            },
            sbom: None,
            description: Some("Command-line JSON processor".to_string()),
            homepage: Some("https://jqlang.github.io/jq/".to_string()),
            license: Some("MIT".to_string()),
        };

        let jq_160 = VersionEntry {
            revision: 2,
            arch: "arm64".to_string(),
            sha256: "def456".to_string(),
            download_url: "https://cdn.example.com/jq-1.6.0-2.arm64.sp".to_string(),
            minisig_url: "https://cdn.example.com/jq-1.6.0-2.arm64.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["oniguruma>=6.0.0".to_string()],
                build: vec![],
            },
            sbom: None,
            description: Some("Command-line JSON processor".to_string()),
            homepage: Some("https://jqlang.github.io/jq/".to_string()),
            license: Some("MIT".to_string()),
        };

        index.add_version("jq".to_string(), "1.7.0".to_string(), jq_170);
        index.add_version("jq".to_string(), "1.6.0".to_string(), jq_160);

        // Add curl package
        let curl_850 = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            sha256: "789xyz".to_string(),
            download_url: "https://cdn.example.com/curl-8.5.0-1.arm64.sp".to_string(),
            minisig_url: "https://cdn.example.com/curl-8.5.0-1.arm64.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["openssl>=3.0.0".to_string(), "zlib~=1.2.0".to_string()],
                build: vec!["pkg-config>=0.29".to_string()],
            },
            sbom: Some(SbomInfo {
                spdx: SbomEntry {
                    url: "https://cdn.example.com/curl-8.5.0-1.arm64.sbom.spdx.json".to_string(),
                    sha256: "sbom123".to_string(),
                },
                cyclonedx: None,
            }),
            description: Some("Command line HTTP client".to_string()),
            homepage: Some("https://curl.se".to_string()),
            license: Some("MIT".to_string()),
        };

        index.add_version("curl".to_string(), "8.5.0".to_string(), curl_850);

        index
    }

    #[tokio::test]
    async fn test_index_manager() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        // Create and load test index
        let index = create_test_index();
        let json = index.to_json().unwrap();

        manager.load(Some(&json)).await.unwrap();

        // Test metadata
        let metadata = manager.metadata().unwrap();
        assert_eq!(metadata.version, 1);

        // Test search
        assert_eq!(manager.search("j"), vec!["jq"]);
        assert_eq!(manager.search("cur"), vec!["curl"]);
        assert!(manager.search("xyz").is_empty());

        // Test get versions
        let jq_versions = manager.get_package_versions("jq").unwrap();
        assert_eq!(jq_versions.len(), 2);
        // Should be sorted newest first
        assert_eq!(jq_versions[0].sha256, "abc123"); // 1.7.0
        assert_eq!(jq_versions[1].sha256, "def456"); // 1.6.0

        // Test find best version
        let spec = PackageSpec::parse("jq>=1.6.0").unwrap();
        let best = manager.find_best_version(&spec).unwrap();
        assert_eq!(best.sha256, "abc123"); // Should pick 1.7.0

        let spec = PackageSpec::parse("jq==1.6.0").unwrap();
        let best = manager.find_best_version(&spec).unwrap();
        assert_eq!(best.sha256, "def456"); // Should pick 1.6.0

        // Test get specific version
        let version = manager.get_version("curl", "8.5.0").unwrap();
        assert_eq!(version.revision, 1);
        assert!(version.has_sbom());
    }

    #[tokio::test]
    async fn test_index_cache() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_test_index();
        manager.set_index(index);

        // Save to cache
        manager.save_to_cache().await.unwrap();

        // Create new manager and load from cache
        let mut manager2 = IndexManager::new(temp.path());
        manager2.load(None).await.unwrap();

        // Verify loaded correctly
        assert_eq!(manager2.index().unwrap().package_count(), 2);
        assert!(manager2.get_version("jq", "1.7.0").is_some());
    }

    #[test]
    fn test_index_staleness() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        // Create index with old timestamp
        let mut index = Index::new();
        index.metadata.timestamp = chrono::Utc::now() - chrono::Duration::days(10);
        manager.set_index(index);

        // Should be stale with 7 day limit
        assert!(manager.is_stale(7));

        // Should not be stale with 30 day limit
        assert!(!manager.is_stale(30));
    }

    #[tokio::test]
    async fn test_version_constraints() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_test_index();
        manager.set_index(index);

        // Test various constraints
        let test_cases = vec![
            ("jq>=1.7.0", Some("abc123")), // 1.7.0
            ("jq<1.7.0", Some("def456")),  // 1.6.0
            ("jq~=1.6.0", Some("def456")), // 1.6.0 (compatible)
            ("jq>2.0.0", None),            // No match
            ("nonexistent>=1.0.0", None),  // Package doesn't exist
        ];

        for (spec_str, expected_hash) in test_cases {
            let spec = PackageSpec::parse(spec_str).unwrap();
            let result = manager.find_best_version(&spec);

            match expected_hash {
                Some(hash) => {
                    assert!(result.is_some(), "Expected match for {spec_str}");
                    assert_eq!(result.unwrap().sha256, hash);
                }
                None => {
                    assert!(result.is_none(), "Expected no match for {spec_str}");
                }
            }
        }
    }
}

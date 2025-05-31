//! Integration tests for builder crate

#[cfg(test)]
mod tests {
    use sps2_builder::*;
    use sps2_types::Version;
    use tempfile::tempdir;
    use tokio::fs;

    const SIMPLE_RECIPE: &str = r#"
fn metadata(m) {
    m.name("hello")
     .version("1.0.0")
     .description("Simple hello world package");
}

fn build(b) {
    b.install();
}
"#;

    const _COMPLEX_RECIPE: &str = r#"
fn metadata(m) {
    m.name("curl")
     .version("8.5.0")
     .description("Command line HTTP client");

    m.depends_on("openssl>=3.0.0");
    m.depends_on("zlib~=1.2.0");
    m.build_depends_on("pkg-config>=0.29");
    m.build_depends_on("autoconf>=2.71");
}

fn build(b) {
    b.fetch(
        "https://curl.se/download/curl-8.5.0.tar.gz",
        "e5250581a9c032b1b6ed3cf2f9c114c811fc41c954b8cea4d5d5bb2a21ea5e90"
    )
    .autotools(["--with-ssl", "--without-librtmp"])
    .install();
}
"#;

    #[tokio::test]
    async fn test_build_context_creation() {
        let temp = tempdir().unwrap();
        let recipe_path = temp.path().join("test.star");
        fs::write(&recipe_path, SIMPLE_RECIPE).await.unwrap();

        let context = BuildContext::new(
            "test-pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
            recipe_path,
            temp.path().to_path_buf(),
        );

        assert_eq!(context.name, "test-pkg");
        assert_eq!(context.version, Version::parse("1.0.0").unwrap());
        assert_eq!(context.revision, 1);
        assert_eq!(context.arch, "arm64");
        assert_eq!(context.package_filename(), "test-pkg-1.0.0-1.arm64.sp");
    }

    #[tokio::test]
    async fn test_build_context_customization() {
        let temp = tempdir().unwrap();
        let recipe_path = temp.path().join("test.star");

        let context = BuildContext::new(
            "my-pkg".to_string(),
            Version::parse("2.1.0").unwrap(),
            recipe_path,
            temp.path().to_path_buf(),
        )
        .with_revision(3)
        .with_arch("x86_64".to_string());

        assert_eq!(context.revision, 3);
        assert_eq!(context.arch, "x86_64");
        assert_eq!(context.package_filename(), "my-pkg-2.1.0-3.x86_64.sp");
    }

    #[test]
    fn test_build_config_defaults() {
        let config = BuildConfig::default();

        assert!(!config.allow_network);
        assert_eq!(config.max_build_time, Some(3600));
        assert!(config.build_jobs.is_none());
        assert!(config.sbom_config.generate_spdx);
        assert!(!config.sbom_config.generate_cyclonedx);
    }

    #[test]
    fn test_build_config_customization() {
        let config = BuildConfig::with_network()
            .with_timeout(1800)
            .with_jobs(8)
            .with_sbom_config(SbomConfig::with_both_formats());

        assert!(config.allow_network);
        assert_eq!(config.max_build_time, Some(1800));
        assert_eq!(config.build_jobs, Some(8));
        assert!(config.sbom_config.generate_spdx);
        assert!(config.sbom_config.generate_cyclonedx);
    }

    #[test]
    fn test_sbom_config() {
        let config = SbomConfig::default();
        assert!(config.generate_spdx);
        assert!(!config.generate_cyclonedx);
        assert!(!config.exclude_patterns.is_empty());
        assert!(config.include_dependencies);

        let both_config = SbomConfig::with_both_formats()
            .exclude("*.test".to_string())
            .include_dependencies(false);

        assert!(both_config.generate_spdx);
        assert!(both_config.generate_cyclonedx);
        assert!(both_config.exclude_patterns.contains(&"*.test".to_string()));
        assert!(!both_config.include_dependencies);
    }

    #[tokio::test]
    async fn test_sbom_generator() {
        let generator = SbomGenerator::new();

        // Test Syft availability (may not be installed in test environment)
        let available = generator.check_syft_available().await.unwrap_or(false);

        // If Syft is not available, that's expected in test environment
        // In production, Syft would be a required dependency
        println!("Syft available: {available}");
    }

    #[tokio::test]
    async fn test_sbom_files() {
        let mut files = SbomFiles::new();
        assert!(!files.has_files());

        // Simulate SPDX file generation
        files.spdx_path = Some("/tmp/test.spdx.json".into());
        files.spdx_hash = Some("abc123".to_string());
        assert!(files.has_files());

        // Add CycloneDX
        files.cyclonedx_path = Some("/tmp/test.cdx.json".into());
        files.cyclonedx_hash = Some("def456".to_string());
        assert!(files.has_files());
    }

    #[tokio::test]
    async fn test_builder_api() {
        let temp = tempdir().unwrap();
        let mut api = BuilderApi::new(temp.path().to_path_buf()).unwrap();

        // Test configuration
        let _ = api
            .allow_network(true)
            .auto_sbom(false)
            .sbom_excludes(vec!["*.debug".to_string()]);

        let (sbom_enabled, excludes) = api.sbom_config();
        assert!(!sbom_enabled);
        assert_eq!(excludes, &["*.debug"]);
    }

    #[tokio::test]
    async fn test_builder_creation() {
        let _builder = Builder::new();
        // Can't test private config fields directly, just ensure creation works

        let config = BuildConfig::with_network().with_jobs(4);
        let _custom_builder = Builder::with_config(config);
        // Config is verified through behavior in actual builds
    }

    #[tokio::test]
    async fn test_build_environment() {
        let temp = tempdir().unwrap();
        let recipe_path = temp.path().join("test.star");
        fs::write(&recipe_path, SIMPLE_RECIPE).await.unwrap();

        let context = BuildContext::new(
            "test-pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
            recipe_path,
            temp.path().to_path_buf(),
        );

        let build_root = temp.path(); // Use temp directory as build root for test
        let env = BuildEnvironment::new(context, build_root).unwrap();

        // Verify environment setup
        assert!(env.env_vars().contains_key("PREFIX"));
        assert!(env.env_vars().contains_key("JOBS"));

        // Verify paths
        assert!(env.build_prefix().to_string_lossy().contains("test-pkg"));
        assert!(env.staging_dir().to_string_lossy().contains("stage"));
    }

    #[tokio::test]
    async fn test_build_result() {
        let temp = tempdir().unwrap();
        let package_path = temp.path().join("test-1.0.0-1.arm64.sp");

        let mut result = BuildResult::new(package_path.clone());
        assert_eq!(result.package_path, package_path);
        assert!(result.sbom_files.is_empty());
        assert!(result.build_log.is_empty());

        // Add SBOM files
        result.add_sbom_file("/tmp/test.spdx.json".into());
        result.add_sbom_file("/tmp/test.cdx.json".into());
        assert_eq!(result.sbom_files.len(), 2);

        // Set build log
        result.set_build_log("Build completed successfully".to_string());
        assert!(!result.build_log.is_empty());
    }

    // Note: Full integration test with actual package building would require:
    // 1. A real resolver with package index
    // 2. A package store for output
    // 3. Syft installed for SBOM generation
    // 4. Proper recipe execution environment
    //
    // For now, we test the individual components and configuration
}

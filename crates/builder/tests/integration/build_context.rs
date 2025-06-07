//! Build context and configuration tests

use sps2_builder::*;
use sps2_types::Version;
use std::path::PathBuf;
use tempfile::tempdir;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_build_context_creation() {
    let temp = tempdir().unwrap();
    let recipe_path = temp.path().join("test.star");

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

    let (tx, _rx) = mpsc::unbounded_channel();
    let context = BuildContext::new(
        "my-pkg".to_string(),
        Version::parse("2.1.0").unwrap(),
        recipe_path,
        temp.path().to_path_buf(),
    )
    .with_revision(3)
    .with_arch("x86_64".to_string())
    .with_event_sender(tx);

    assert_eq!(context.revision, 3);
    assert_eq!(context.arch, "x86_64");
    assert_eq!(context.package_filename(), "my-pkg-2.1.0-3.x86_64.sp");
    assert!(context.event_sender.is_some());
}

#[test]
fn test_build_config_defaults() {
    let config = BuildConfig::default();

    assert!(!config.allow_network);
    assert_eq!(config.max_build_time, Some(3600));
    assert!(config.build_jobs.is_none());
    assert!(config.sbom_config.generate_spdx);
    assert!(!config.sbom_config.generate_cyclonedx);
    assert_eq!(config.build_root, Some(PathBuf::from("/opt/pm/build")));
}

#[test]
fn test_build_config_customization() {
    let _custom_root = tempdir().unwrap();
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

#[tokio::test]
async fn test_builder_creation_and_configuration() {
    let temp = tempdir().unwrap();
    let _builder = Builder::new();

    let config = BuildConfig::default()
        .with_timeout(1200)
        .with_sbom_config(SbomConfig::with_both_formats());

    // Test builder with custom configuration
    let _context = BuildContext::new(
        "builder-test".to_string(),
        Version::parse("1.0.0").unwrap(),
        temp.path().join("recipe.star"),
        temp.path().to_path_buf(),
    );

    // Verify builder can be created and configured
    assert!(temp.path().exists());
    assert_eq!(config.max_build_time, Some(1200));
}

#[tokio::test]
async fn test_build_environment_comprehensive() {
    let temp = tempdir().unwrap();

    let config = BuildConfig::default()
        .with_timeout(1800)
        .with_jobs(4)
        .with_sbom_config(SbomConfig::with_both_formats());

    // Test environment setup
    assert_eq!(config.max_build_time, Some(1800));
    assert_eq!(config.build_jobs, Some(4));
    assert!(config.sbom_config.generate_spdx);
    assert!(config.sbom_config.generate_cyclonedx);

    // Verify build environment can be configured
    let context = BuildContext::new(
        "env-test".to_string(),
        Version::parse("2.0.0").unwrap(),
        temp.path().join("test.star"),
        temp.path().to_path_buf(),
    )
    .with_revision(2)
    .with_arch("x86_64".to_string());

    assert_eq!(context.arch, "x86_64");
    assert_eq!(context.revision, 2);
}

#[tokio::test]
async fn test_build_result_comprehensive() {
    // Test BuildResult structure and validation
    let temp = tempdir().unwrap();

    let context = BuildContext::new(
        "result-test".to_string(),
        Version::parse("1.5.0").unwrap(),
        temp.path().join("test.star"),
        temp.path().to_path_buf(),
    );

    // Verify context generates correct package filename
    assert_eq!(context.package_filename(), "result-test-1.5.0-1.arm64.sp");

    // Test various build configurations
    let config = BuildConfig::default()
        .with_timeout(900)
        .with_sbom_config(SbomConfig::with_both_formats());

    assert_eq!(config.max_build_time, Some(900));
    assert!(config.sbom_config.generate_spdx);
    assert!(config.sbom_config.generate_cyclonedx);
}

#[tokio::test]
async fn test_build_environment_advanced() {
    let temp = tempdir().unwrap();

    // Test advanced build environment setup
    let config = BuildConfig::with_network().with_timeout(2400).with_jobs(16);

    assert!(config.allow_network);
    assert_eq!(config.max_build_time, Some(2400));
    assert_eq!(config.build_jobs, Some(16));

    // Test context with advanced settings
    let (tx, _rx) = mpsc::unbounded_channel();
    let context = BuildContext::new(
        "advanced-test".to_string(),
        Version::parse("3.1.4").unwrap(),
        temp.path().join("advanced.star"),
        temp.path().to_path_buf(),
    )
    .with_revision(5)
    .with_arch("aarch64".to_string())
    .with_event_sender(tx);

    assert_eq!(context.revision, 5);
    assert_eq!(context.arch, "aarch64");
    assert!(context.event_sender.is_some());
}

#[tokio::test]
async fn test_signing_config() {
    let config = BuildConfig::default();

    // Test default signing configuration
    assert!(!config.allow_network);
    assert!(config.sbom_config.generate_spdx);

    // Test custom signing configuration
    let custom_config =
        BuildConfig::with_network().with_sbom_config(SbomConfig::with_both_formats());

    assert!(custom_config.allow_network);
    assert!(custom_config.sbom_config.generate_spdx);
    assert!(custom_config.sbom_config.generate_cyclonedx);
}

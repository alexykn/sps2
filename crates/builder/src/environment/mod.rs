//! Build environment management
//!
//! This module provides isolated build environments for package building.
//! It manages directory structure, environment variables, dependency installation,
//! command execution, and environment isolation verification.

mod core;
mod dependencies;
mod directories;
mod execution;
mod hermetic;
mod isolation;
mod sandbox;
mod types;
mod variables;

// Re-export public API
pub use core::BuildEnvironment;
pub use types::{BuildCommandResult, BuildResult};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BuildContext;
    use sps2_types::Version;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_environment_creation() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "test-pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        let build_root = temp.path(); // Use temp directory as build root for test
        let env = BuildEnvironment::new(context, build_root).unwrap();

        assert_eq!(env.context.name, "test-pkg");
        assert!(env.env_vars().contains_key("PREFIX"));
        assert!(env.env_vars().contains_key("JOBS"));
    }

    #[tokio::test]
    async fn test_environment_initialization() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "test-pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        let build_root = temp.path(); // Use temp directory as build root for test
        let env = BuildEnvironment::new(context, build_root).unwrap();

        // This would normally require /opt/pm/build to exist
        // For testing, just verify the structure
        assert!(env
            .build_prefix()
            .display()
            .to_string()
            .contains("test-pkg"));
        assert!(env.staging_dir().display().to_string().contains("stage"));
    }

    #[test]
    fn test_cpu_count() {
        let count = BuildEnvironment::cpu_count();
        assert!(count > 0);
        assert!(count <= num_cpus::get());
    }

    // TODO: Re-enable this test when CI permissions are fixed
    // This test fails in GitHub Actions CI due to permission denied errors
    // when creating directories. It works locally but the CI environment
    // has different filesystem permissions that prevent directory creation.
    #[ignore]
    #[tokio::test]
    async fn test_environment_isolation() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "isolated-test".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        let build_root = temp.path(); // Use temp directory as build root for test
        let mut env = BuildEnvironment::new(context, build_root).unwrap();
        env.initialize().await.unwrap();

        // Verify isolation setup
        assert!(env.verify_isolation().is_ok());

        // Check that essential environment variables are set
        assert!(env.env_vars().contains_key("PREFIX"));
        assert!(env.env_vars().contains_key("DESTDIR"));
        assert!(env.env_vars().contains_key("JOBS"));
        assert!(env.env_vars().contains_key("PATH"));

        // Verify PATH is set (it won't start with deps_bin until build deps are set up)
        let path = env.env_vars().get("PATH").unwrap();
        assert!(!path.is_empty());

        // Verify environment summary includes key information
        let summary = env.environment_summary();
        assert!(summary.contains_key("build_prefix"));
        assert!(summary.contains_key("deps_prefix"));
        assert!(summary.contains_key("staging_dir"));
        assert!(summary.contains_key("package_name"));
        assert!(summary.contains_key("PATH"));
    }

    // TODO: Re-enable this test when CI permissions are fixed
    // This test fails in GitHub Actions CI due to permission denied errors
    // when creating directories. It works locally but the CI environment
    // has different filesystem permissions that prevent directory creation.
    #[ignore]
    #[tokio::test]
    async fn test_clean_environment_setup() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "clean-test".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        let build_root = temp.path(); // Use temp directory as build root for test
        let mut env = BuildEnvironment::new(context, build_root).unwrap();

        // Set some potentially harmful environment variables in the process
        std::env::set_var("LDFLAGS", "-L/some/bad/path");
        std::env::set_var("PKG_CONFIG_PATH", "/bad/pkgconfig");

        env.initialize().await.unwrap();

        // The BuildEnvironment creates its own isolated environment
        // It doesn't copy problematic variables from the process environment
        // Instead it sets up clean versions with only the deps prefix
        let ldflags = env.env_vars().get("LDFLAGS").unwrap();
        assert_eq!(ldflags, &format!("-L{}/lib", env.deps_prefix.display()));

        // PKG_CONFIG_PATH is not set initially, only when build deps are set up
        assert!(!env.env_vars().contains_key("PKG_CONFIG_PATH"));

        // Setup build deps environment to get PKG_CONFIG_PATH
        env.setup_build_deps_environment();
        let pkg_config = env.env_vars().get("PKG_CONFIG_PATH").unwrap();
        assert_eq!(
            pkg_config,
            &format!("{}/lib/pkgconfig", env.deps_prefix.display())
        );

        // Clean up test environment variables
        std::env::remove_var("LDFLAGS");
        std::env::remove_var("PKG_CONFIG_PATH");
    }
}

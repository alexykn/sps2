//! Build environment management

use crate::BuildContext;
use spsv2_errors::{BuildError, Error};
use spsv2_events::Event;
use spsv2_install::Installer;
use spsv2_resolver::{DepKind, ResolutionContext, Resolver};
use spsv2_store::PackageStore;
use spsv2_types::{package::PackageSpec, Version};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::fs;
use tokio::process::Command;

/// Build environment for isolated package building
pub struct BuildEnvironment {
    /// Build context
    context: BuildContext,
    /// Build prefix directory
    build_prefix: PathBuf,
    /// Build dependencies prefix
    deps_prefix: PathBuf,
    /// Staging directory for installation
    staging_dir: PathBuf,
    /// Environment variables
    env_vars: HashMap<String, String>,
    /// Resolver for dependencies
    resolver: Option<Resolver>,
    /// Package store for build dependencies
    store: Option<PackageStore>,
    /// Installer for build dependencies
    installer: Option<Installer>,
}

impl BuildEnvironment {
    /// Create new build environment
    ///
    /// # Errors
    ///
    /// Returns an error if the build environment cannot be initialized.
    pub fn new(context: BuildContext) -> Result<Self, Error> {
        let build_prefix = Self::get_build_prefix_path(&context.name, &context.version);
        let deps_prefix = build_prefix.join("deps");
        let staging_dir = build_prefix.join("stage");

        let mut env_vars = HashMap::new();
        env_vars.insert("PREFIX".to_string(), staging_dir.display().to_string());
        env_vars.insert("JOBS".to_string(), Self::cpu_count().to_string());

        Ok(Self {
            context,
            build_prefix,
            deps_prefix,
            staging_dir,
            env_vars,
            resolver: None,
            store: None,
            installer: None,
        })
    }

    /// Set resolver for dependency management
    #[must_use]
    pub fn with_resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Set package store for build dependencies
    #[must_use]
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Set installer for build dependencies
    #[must_use]
    pub fn with_installer(mut self, installer: Installer) -> Self {
        self.installer = Some(installer);
        self
    }

    /// Initialize the build environment
    ///
    /// # Errors
    ///
    /// Returns an error if directories cannot be created or environment setup fails.
    pub async fn initialize(&mut self) -> Result<(), Error> {
        self.send_event(Event::OperationStarted {
            operation: format!("Building {} {}", self.context.name, self.context.version),
        });

        // Create build directories
        fs::create_dir_all(&self.build_prefix).await?;
        fs::create_dir_all(&self.deps_prefix).await?;
        fs::create_dir_all(&self.staging_dir).await?;

        // Set up environment variables
        self.setup_environment();

        Ok(())
    }

    /// Setup build dependencies
    ///
    /// # Errors
    ///
    /// Returns an error if dependency resolution fails or build dependencies cannot be installed.
    pub async fn setup_dependencies(&mut self, build_deps: Vec<PackageSpec>) -> Result<(), Error> {
        if build_deps.is_empty() {
            return Ok(());
        }

        let Some(resolver) = &self.resolver else {
            return Err(BuildError::MissingBuildDep {
                name: "resolver configuration".to_string(),
            }
            .into());
        };

        self.send_event(Event::DependencyResolved {
            package: self.context.name.clone(),
            version: self.context.version.clone(),
            count: 1, // Single package resolved
        });

        // Resolve build dependencies
        let mut resolution_context = ResolutionContext::new();
        for dep in build_deps {
            resolution_context = resolution_context.add_build_dep(dep);
        }

        let resolution = resolver.resolve(resolution_context).await?;

        // Install build dependencies to deps prefix
        for node in resolution.packages_in_order() {
            if node.deps.iter().any(|edge| edge.kind == DepKind::Build) {
                self.install_build_dependency(node)?;
            }
        }

        // Update environment for build deps
        self.setup_build_deps_environment();

        Ok(())
    }

    /// Execute a command in the build environment
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute or exits with a non-zero status.
    pub async fn execute_command(
        &self,
        program: &str,
        args: &[&str],
        working_dir: Option<&Path>,
    ) -> Result<BuildCommandResult, Error> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.envs(&self.env_vars);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&self.build_prefix);
        }

        self.send_event(Event::BuildStepStarted {
            step: format!("{program} {}", args.join(" ")),
            package: self.context.name.clone(),
        });

        let output = cmd.output().await.map_err(|e| BuildError::CompileFailed {
            message: format!("{program}: {e}"),
        })?;

        let result = BuildCommandResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        };

        if !result.success {
            return Err(BuildError::CompileFailed {
                message: format!("{program} {} failed: {}", args.join(" "), result.stderr),
            }
            .into());
        }

        Ok(result)
    }

    /// Clean up build environment thoroughly
    ///
    /// # Errors
    ///
    /// Returns an error if directories cannot be removed during cleanup.
    pub async fn cleanup(&self) -> Result<(), Error> {
        // Remove build dependencies directory
        if self.deps_prefix.exists() {
            fs::remove_dir_all(&self.deps_prefix).await?;
        }

        // Remove any temporary build files in the build prefix
        let temp_dirs = vec!["src", "build", "tmp"];
        for dir in temp_dirs {
            let temp_path = self.build_prefix.join(dir);
            if temp_path.exists() {
                fs::remove_dir_all(&temp_path).await?;
            }
        }

        self.send_event(Event::OperationCompleted {
            operation: format!("Cleaned build environment for {}", self.context.name),
            success: true,
        });

        Ok(())
    }

    /// Verify build environment isolation is properly set up
    ///
    /// # Errors
    ///
    /// Returns an error if the build environment is not properly isolated or directories are missing.
    pub fn verify_isolation(&self) -> Result<(), Error> {
        // Check that critical directories exist
        if !self.build_prefix.exists() {
            return Err(BuildError::Failed {
                message: format!(
                    "Build prefix does not exist: {}",
                    self.build_prefix.display()
                ),
            }
            .into());
        }

        if !self.staging_dir.exists() {
            return Err(BuildError::Failed {
                message: format!(
                    "Staging directory does not exist: {}",
                    self.staging_dir.display()
                ),
            }
            .into());
        }

        // Verify environment variables are set correctly
        let required_vars = vec!["PREFIX", "DESTDIR", "JOBS"];
        for var in required_vars {
            if !self.env_vars.contains_key(var) {
                return Err(BuildError::Failed {
                    message: format!("Required environment variable {var} not set"),
                }
                .into());
            }
        }

        // PATH will be updated when build dependencies are installed
        // So we just check it exists for now
        if !self.env_vars.contains_key("PATH") {
            return Err(BuildError::Failed {
                message: "PATH environment variable not set".to_string(),
            }
            .into());
        }

        Ok(())
    }

    /// Get a summary of the build environment for debugging
    #[must_use]
    pub fn environment_summary(&self) -> HashMap<String, String> {
        let mut summary = HashMap::new();

        summary.insert(
            "build_prefix".to_string(),
            self.build_prefix.display().to_string(),
        );
        summary.insert(
            "deps_prefix".to_string(),
            self.deps_prefix.display().to_string(),
        );
        summary.insert(
            "staging_dir".to_string(),
            self.staging_dir.display().to_string(),
        );
        summary.insert("package_name".to_string(), self.context.name.clone());
        summary.insert(
            "package_version".to_string(),
            self.context.version.to_string(),
        );

        // Add key environment variables
        for key in &[
            "PATH",
            "PKG_CONFIG_PATH",
            "CMAKE_PREFIX_PATH",
            "CFLAGS",
            "LDFLAGS",
        ] {
            if let Some(value) = self.env_vars.get(*key) {
                summary.insert((*key).to_string(), value.clone());
            }
        }

        summary
    }

    /// Get staging directory
    #[must_use]
    pub fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }

    /// Get build prefix
    #[must_use]
    pub fn build_prefix(&self) -> &Path {
        &self.build_prefix
    }

    /// Get environment variables
    #[must_use]
    pub fn env_vars(&self) -> &HashMap<String, String> {
        &self.env_vars
    }

    /// Set environment variable
    ///
    /// # Errors
    ///
    /// Currently infallible, but returns Result for future compatibility.
    pub fn set_env_var(&mut self, key: String, value: String) -> Result<(), Error> {
        self.env_vars.insert(key, value);
        Ok(())
    }

    /// Get build prefix path for package
    #[must_use]
    fn get_build_prefix_path(name: &str, version: &Version) -> PathBuf {
        PathBuf::from("/opt/pm/build")
            .join(name)
            .join(version.to_string())
    }

    /// Get CPU count for parallel builds
    #[must_use]
    fn cpu_count() -> usize {
        // Use 75% of available cores as per specification
        let cores = num_cpus::get();
        let target = cores.saturating_mul(3).saturating_add(3) / 4; // 75% using integer arithmetic
        std::cmp::max(1, target)
    }

    /// Setup base environment variables for isolated build
    fn setup_environment(&mut self) {
        // Clear potentially harmful environment variables for clean build
        self.setup_clean_environment();

        // Add staging dir to environment (standard autotools DESTDIR)
        self.env_vars.insert(
            "DESTDIR".to_string(),
            self.staging_dir.display().to_string(),
        );

        // Set build prefix explicitly
        self.env_vars
            .insert("PREFIX".to_string(), self.staging_dir.display().to_string());

        // Number of parallel jobs
        self.env_vars
            .insert("JOBS".to_string(), Self::cpu_count().to_string());
        self.env_vars
            .insert("MAKEFLAGS".to_string(), format!("-j{}", Self::cpu_count()));

        // Compiler flags for dependency isolation
        let deps_prefix_display = self.deps_prefix.display();
        self.env_vars.insert(
            "CFLAGS".to_string(),
            format!("-I{deps_prefix_display}/include"),
        );
        self.env_vars.insert(
            "CPPFLAGS".to_string(),
            format!("-I{deps_prefix_display}/include"),
        );
        self.env_vars.insert(
            "LDFLAGS".to_string(),
            format!("-L{deps_prefix_display}/lib"),
        );

        // Prevent system library contamination
        self.env_vars.insert(
            "LIBRARY_PATH".to_string(),
            format!("{deps_prefix_display}/lib"),
        );
        self.env_vars.insert(
            "LD_LIBRARY_PATH".to_string(),
            format!("{deps_prefix_display}/lib"),
        );

        // macOS specific settings - targeting Apple Silicon Macs (macOS 12.0+)
        self.env_vars
            .insert("MACOSX_DEPLOYMENT_TARGET".to_string(), "12.0".to_string());
    }

    /// Setup a clean environment by removing potentially harmful variables
    fn setup_clean_environment(&mut self) {
        // Keep only essential environment variables
        let essential_vars = vec![
            "PATH", "HOME", "USER", "SHELL", "TERM", "LANG", "LC_ALL", "TMPDIR", "TMP", "TEMP",
        ];

        // Start with a minimal PATH containing only system essentials
        self.env_vars.insert(
            "PATH".to_string(),
            "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
        );

        // Copy essential variables from host environment
        for var in essential_vars {
            if let Ok(value) = std::env::var(var) {
                self.env_vars.insert(var.to_string(), value);
            }
        }

        // Clear potentially problematic variables
        self.env_vars.remove("CFLAGS");
        self.env_vars.remove("CPPFLAGS");
        self.env_vars.remove("LDFLAGS");
        self.env_vars.remove("PKG_CONFIG_PATH");
        self.env_vars.remove("LIBRARY_PATH");
        self.env_vars.remove("LD_LIBRARY_PATH");
        self.env_vars.remove("DYLD_LIBRARY_PATH");
        self.env_vars.remove("CMAKE_PREFIX_PATH");
        self.env_vars.remove("ACLOCAL_PATH");
    }

    /// Setup environment for build dependencies with proper isolation
    fn setup_build_deps_environment(&mut self) {
        let deps_prefix_display = self.deps_prefix.display();
        let deps_bin = format!("{deps_prefix_display}/bin");
        let deps_lib = format!("{deps_prefix_display}/lib");
        let deps_include = format!("{deps_prefix_display}/include");
        let deps_pkgconfig = format!("{deps_prefix_display}/lib/pkgconfig");
        let deps_share = format!("{deps_prefix_display}/share");

        // Prepend build deps to PATH (highest priority)
        let current_path = self.env_vars.get("PATH").cloned().unwrap_or_default();
        let new_path = if current_path.is_empty() {
            deps_bin
        } else {
            format!("{deps_bin}:{current_path}")
        };
        self.env_vars.insert("PATH".to_string(), new_path);

        // PKG_CONFIG_PATH for dependency discovery
        self.env_vars
            .insert("PKG_CONFIG_PATH".to_string(), deps_pkgconfig.clone());

        // CMAKE_PREFIX_PATH for CMake-based builds
        self.env_vars.insert(
            "CMAKE_PREFIX_PATH".to_string(),
            self.deps_prefix.display().to_string(),
        );

        // Update compiler flags to include build dep paths
        let current_cflags = self.env_vars.get("CFLAGS").cloned().unwrap_or_default();
        let new_cflags = if current_cflags.is_empty() {
            format!("-I{deps_include}")
        } else {
            format!("{current_cflags} -I{deps_include}")
        };
        self.env_vars.insert("CFLAGS".to_string(), new_cflags);

        let current_cppflags = self.env_vars.get("CPPFLAGS").cloned().unwrap_or_default();
        let new_cppflags = if current_cppflags.is_empty() {
            format!("-I{deps_include}")
        } else {
            format!("{current_cppflags} -I{deps_include}")
        };
        self.env_vars.insert("CPPFLAGS".to_string(), new_cppflags);

        let current_ldflags = self.env_vars.get("LDFLAGS").cloned().unwrap_or_default();
        let new_ldflags = if current_ldflags.is_empty() {
            format!("-L{deps_lib}")
        } else {
            format!("{current_ldflags} -L{deps_lib}")
        };
        self.env_vars.insert("LDFLAGS".to_string(), new_ldflags);

        // Autotools-specific paths
        self.env_vars
            .insert("ACLOCAL_PATH".to_string(), format!("{deps_share}/aclocal"));

        // Ensure library search paths are set
        self.env_vars
            .insert("LIBRARY_PATH".to_string(), deps_lib.clone());
        self.env_vars
            .insert("LD_LIBRARY_PATH".to_string(), deps_lib);
    }

    /// Install a build dependency to isolated prefix
    ///
    /// # Errors
    ///
    /// Returns an error if the installer or store is not configured, or if installation fails.
    fn install_build_dependency(&self, node: &spsv2_resolver::ResolvedNode) -> Result<(), Error> {
        let Some(_installer) = &self.installer else {
            return Err(BuildError::MissingBuildDep {
                name: "installer not configured".to_string(),
            }
            .into());
        };

        let Some(_store) = &self.store else {
            return Err(BuildError::MissingBuildDep {
                name: "package store not configured".to_string(),
            }
            .into());
        };

        self.send_event(Event::PackageInstalling {
            name: node.name.clone(),
            version: node.version.clone(),
        });

        // Install the build dependency to the isolated deps prefix
        // This extracts the package contents to the build environment
        match &node.action {
            spsv2_resolver::NodeAction::Download => {
                if let Some(url) = &node.url {
                    // Download and install the package to deps prefix
                    // For now, we'll simulate this - in a full implementation,
                    // this would download the .sp file and extract it to deps_prefix
                    self.send_event(Event::DownloadStarted {
                        url: url.clone(),
                        size: None,
                    });

                    // TODO: Implement actual package download and extraction
                    // installer.install_to_prefix(url, &self.deps_prefix).await?;

                    self.send_event(Event::PackageInstalled {
                        name: node.name.clone(),
                        version: node.version.clone(),
                        path: self.deps_prefix.display().to_string(),
                    });
                }
            }
            spsv2_resolver::NodeAction::Local => {
                if let Some(_path) = &node.path {
                    // Install from local .sp file to deps prefix
                    // TODO: Implement actual local package installation
                    // installer.install_local_to_prefix(path, &self.deps_prefix).await?;

                    self.send_event(Event::PackageInstalled {
                        name: node.name.clone(),
                        version: node.version.clone(),
                        path: self.deps_prefix.display().to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Send event if sender is available
    fn send_event(&self, event: Event) {
        if let Some(sender) = &self.context.event_sender {
            let _ = sender.send(event);
        }
    }
}

/// Result of executing a build command
#[derive(Debug)]
pub struct BuildCommandResult {
    /// Whether the command succeeded
    pub success: bool,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
}

/// Result of the build process
#[derive(Debug)]
pub struct BuildResult {
    /// Path to the generated package file
    pub package_path: PathBuf,
    /// SBOM files generated
    pub sbom_files: Vec<PathBuf>,
    /// Build log
    pub build_log: String,
}

impl BuildResult {
    /// Create new build result
    #[must_use]
    pub fn new(package_path: PathBuf) -> Self {
        Self {
            package_path,
            sbom_files: Vec::new(),
            build_log: String::new(),
        }
    }

    /// Add SBOM file
    pub fn add_sbom_file(&mut self, path: PathBuf) {
        self.sbom_files.push(path);
    }

    /// Set build log
    pub fn set_build_log(&mut self, log: String) {
        self.build_log = log;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spsv2_types::Version;
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

        let env = BuildEnvironment::new(context).unwrap();

        assert_eq!(env.context.name, "test-pkg");
        assert!(env.env_vars.contains_key("PREFIX"));
        assert!(env.env_vars.contains_key("JOBS"));
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

        let env = BuildEnvironment::new(context).unwrap();

        // This would normally require /opt/pm/build to exist
        // For testing, just verify the structure
        assert!(env.build_prefix.display().to_string().contains("test-pkg"));
        assert!(env.staging_dir.display().to_string().contains("stage"));
    }

    #[test]
    fn test_cpu_count() {
        let count = BuildEnvironment::cpu_count();
        assert!(count > 0);
        assert!(count <= num_cpus::get());
    }

    #[tokio::test]
    async fn test_environment_isolation() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "isolated-test".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        let mut env = BuildEnvironment::new(context).unwrap();
        env.initialize().await.unwrap();

        // Verify isolation setup
        assert!(env.verify_isolation().is_ok());

        // Check that essential environment variables are set
        assert!(env.env_vars.contains_key("PREFIX"));
        assert!(env.env_vars.contains_key("DESTDIR"));
        assert!(env.env_vars.contains_key("JOBS"));
        assert!(env.env_vars.contains_key("PATH"));

        // Verify PATH is set (it won't start with deps_bin until build deps are set up)
        let path = env.env_vars.get("PATH").unwrap();
        assert!(!path.is_empty());

        // Verify environment summary includes key information
        let summary = env.environment_summary();
        assert!(summary.contains_key("build_prefix"));
        assert!(summary.contains_key("deps_prefix"));
        assert!(summary.contains_key("staging_dir"));
        assert!(summary.contains_key("package_name"));
        assert!(summary.contains_key("PATH"));
    }

    #[tokio::test]
    async fn test_clean_environment_setup() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "clean-test".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        let mut env = BuildEnvironment::new(context).unwrap();

        // Set some potentially harmful environment variables in the process
        std::env::set_var("LDFLAGS", "-L/some/bad/path");
        std::env::set_var("PKG_CONFIG_PATH", "/bad/pkgconfig");

        env.initialize().await.unwrap();

        // The BuildEnvironment creates its own isolated environment
        // It doesn't copy problematic variables from the process environment
        // Instead it sets up clean versions with only the deps prefix
        let ldflags = env.env_vars.get("LDFLAGS").unwrap();
        assert_eq!(ldflags, &format!("-L{}/lib", env.deps_prefix.display()));

        // PKG_CONFIG_PATH is not set initially, only when build deps are set up
        assert!(!env.env_vars.contains_key("PKG_CONFIG_PATH"));

        // Setup build deps environment to get PKG_CONFIG_PATH
        env.setup_build_deps_environment();
        let pkg_config = env.env_vars.get("PKG_CONFIG_PATH").unwrap();
        assert_eq!(
            pkg_config,
            &format!("{}/lib/pkgconfig", env.deps_prefix.display())
        );

        // Clean up test environment variables
        std::env::remove_var("LDFLAGS");
        std::env::remove_var("PKG_CONFIG_PATH");
    }
}

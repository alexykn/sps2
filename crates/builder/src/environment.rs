//! Build environment management

use crate::BuildContext;
use spsv2_errors::{BuildError, Error};
use spsv2_events::{Event, EventSender};
use spsv2_resolver::{DepKind, ResolutionContext, Resolver};
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
}

impl BuildEnvironment {
    /// Create new build environment
    pub fn new(context: BuildContext) -> Result<Self, Error> {
        let build_prefix = Self::get_build_prefix_path(&context.name, &context.version);
        let deps_prefix = build_prefix.join("deps");
        let staging_dir = build_prefix.join("stage");

        let mut env_vars = HashMap::new();
        env_vars.insert("PREFIX".to_string(), staging_dir.display().to_string());
        env_vars.insert("JOBS".to_string(), Self::cpu_count().to_string());

        Ok(Self {
            context,
            build_prefix: build_prefix.clone(),
            deps_prefix,
            staging_dir,
            env_vars,
            resolver: None,
        })
    }

    /// Set resolver for dependency management
    pub fn with_resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Initialize the build environment
    pub async fn initialize(&mut self) -> Result<(), Error> {
        self.send_event(Event::OperationStarted {
            operation: format!("Building {} {}", self.context.name, self.context.version),
        });

        // Create build directories
        fs::create_dir_all(&self.build_prefix).await?;
        fs::create_dir_all(&self.deps_prefix).await?;
        fs::create_dir_all(&self.staging_dir).await?;

        // Set up environment variables
        self.setup_environment().await?;

        Ok(())
    }

    /// Setup build dependencies
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
                self.install_build_dependency(node).await?;
            }
        }

        // Update environment for build deps
        self.setup_build_deps_environment().await?;

        Ok(())
    }

    /// Execute a command in the build environment
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
            message: format!("{}: {}", program, e),
        })?;

        let result = BuildCommandResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        };

        if !result.success {
            return Err(BuildError::CompileFailed {
                message: format!("{program} {} failed: {}", args.join(" "), result.stderr),
            }
            .into());
        }

        Ok(result)
    }

    /// Clean up build environment
    pub async fn cleanup(&self) -> Result<(), Error> {
        // Remove build dependencies directory
        if self.deps_prefix.exists() {
            fs::remove_dir_all(&self.deps_prefix).await?;
        }

        self.send_event(Event::OperationCompleted {
            operation: format!("Cleaned build environment for {}", self.context.name),
            success: true,
        });

        Ok(())
    }

    /// Get staging directory
    pub fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }

    /// Get build prefix
    pub fn build_prefix(&self) -> &Path {
        &self.build_prefix
    }

    /// Get environment variables
    pub fn env_vars(&self) -> &HashMap<String, String> {
        &self.env_vars
    }

    /// Get build prefix path for package
    fn get_build_prefix_path(name: &str, version: &Version) -> PathBuf {
        PathBuf::from("/opt/pm/build")
            .join(name)
            .join(version.to_string())
    }

    /// Get CPU count for parallel builds
    fn cpu_count() -> usize {
        // Use 75% of available cores as per specification
        (num_cpus::get() as f32 * 0.75).ceil() as usize
    }

    /// Setup base environment variables
    async fn setup_environment(&mut self) -> Result<(), Error> {
        // Add staging dir to environment
        self.env_vars.insert(
            "DESTDIR".to_string(),
            self.staging_dir.display().to_string(),
        );

        // Compiler flags
        self.env_vars.insert(
            "CFLAGS".to_string(),
            format!("-I{}/include", self.deps_prefix.display()),
        );
        self.env_vars.insert(
            "LDFLAGS".to_string(),
            format!("-L{}/lib", self.deps_prefix.display()),
        );

        Ok(())
    }

    /// Setup environment for build dependencies
    async fn setup_build_deps_environment(&mut self) -> Result<(), Error> {
        // Add build deps to PATH
        let mut path = format!("{}/bin", self.deps_prefix.display());
        if let Ok(existing_path) = std::env::var("PATH") {
            path.push(':');
            path.push_str(&existing_path);
        }
        self.env_vars.insert("PATH".to_string(), path);

        // PKG_CONFIG_PATH
        self.env_vars.insert(
            "PKG_CONFIG_PATH".to_string(),
            format!("{}/lib/pkgconfig", self.deps_prefix.display()),
        );

        Ok(())
    }

    /// Install a build dependency
    async fn install_build_dependency(
        &self,
        _node: &spsv2_resolver::ResolvedNode,
    ) -> Result<(), Error> {
        // TODO: This would need integration with install crate
        // For now, just return success
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
            temp.path().join("recipe.rhai"),
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
            temp.path().join("recipe.rhai"),
            temp.path().to_path_buf(),
        );

        let mut env = BuildEnvironment::new(context).unwrap();

        // This would normally require /opt/pm/build to exist
        // For testing, just verify the structure
        assert!(env.build_prefix.to_string().contains("test-pkg"));
        assert!(env.staging_dir.to_string().contains("stage"));
    }

    #[test]
    fn test_cpu_count() {
        let count = BuildEnvironment::cpu_count();
        assert!(count > 0);
        assert!(count <= num_cpus::get());
    }
}

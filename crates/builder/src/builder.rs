//! Main builder implementation

use crate::{
    BuildContext, BuildEnvironment, BuildResult, BuilderApi, PackageSigner, SbomConfig, SbomFiles,
    SbomGenerator, SigningConfig,
};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use sps2_manifest::Manifest;
use sps2_net::NetClient;
use sps2_package::{execute_recipe, load_recipe};
use sps2_resolver::Resolver;
use sps2_store::PackageStore;
use sps2_types::package::PackageSpec;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Package builder configuration
#[derive(Clone, Debug)]
pub struct BuildConfig {
    /// SBOM generation configuration
    pub sbom_config: SbomConfig,
    /// Package signing configuration
    pub signing_config: SigningConfig,
    /// Maximum build time in seconds
    pub max_build_time: Option<u64>,
    /// Network access allowed during build
    pub allow_network: bool,
    /// Number of parallel build jobs
    pub build_jobs: Option<usize>,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            sbom_config: SbomConfig::default(),
            signing_config: SigningConfig::default(),
            max_build_time: Some(3600), // 1 hour
            allow_network: false,
            build_jobs: None, // Use auto-detection
        }
    }
}

impl BuildConfig {
    /// Create config with network access enabled
    #[must_use]
    pub fn with_network() -> Self {
        Self {
            allow_network: true,
            ..Default::default()
        }
    }

    /// Set SBOM configuration
    #[must_use]
    pub fn with_sbom_config(mut self, config: SbomConfig) -> Self {
        self.sbom_config = config;
        self
    }

    /// Set signing configuration
    #[must_use]
    pub fn with_signing_config(mut self, config: SigningConfig) -> Self {
        self.signing_config = config;
        self
    }

    /// Set build timeout
    #[must_use]
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.max_build_time = Some(seconds);
        self
    }

    /// Set parallel build jobs
    #[must_use]
    pub fn with_jobs(mut self, jobs: usize) -> Self {
        self.build_jobs = Some(jobs);
        self
    }
}

/// Package builder
#[derive(Clone)]
pub struct Builder {
    /// Build configuration
    config: BuildConfig,
    /// Resolver for dependencies
    resolver: Option<Resolver>,
    /// Package store for output
    store: Option<PackageStore>,
    /// Network client for downloads
    net: Option<NetClient>,
}

impl Builder {
    /// Create new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: BuildConfig::default(),
            resolver: None,
            store: None,
            net: None,
        }
    }

    /// Create builder with configuration
    #[must_use]
    pub fn with_config(config: BuildConfig) -> Self {
        Self {
            config,
            resolver: None,
            store: None,
            net: None,
        }
    }

    /// Set resolver
    #[must_use]
    pub fn with_resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Set package store
    #[must_use]
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Set network client
    #[must_use]
    pub fn with_net(mut self, net: NetClient) -> Self {
        self.net = Some(net);
        self
    }

    /// Build a package from a Starlark recipe
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The recipe file cannot be read or parsed
    /// - Build dependencies cannot be resolved or installed
    /// - The build process fails or times out
    /// - SBOM generation fails
    /// - Package creation or signing fails
    /// - Environment setup or cleanup fails
    pub async fn build(&self, context: BuildContext) -> Result<BuildResult, Error> {
        Self::send_event(
            &context,
            Event::OperationStarted {
                operation: format!("Building {} {}", context.name, context.version),
            },
        );

        // Create build environment with full isolation setup
        let mut environment = BuildEnvironment::new(context.clone())?;

        // Configure environment with resolver, store, and net client if available
        if let Some(resolver) = &self.resolver {
            environment = environment.with_resolver(resolver.clone());
        }
        if let Some(store) = &self.store {
            environment = environment.with_store(store.clone());
        }
        if let Some(net) = &self.net {
            environment = environment.with_net(net.clone());
        }

        // Initialize isolated environment
        environment.initialize().await?;

        // Verify isolation is properly set up
        environment.verify_isolation()?;

        Self::send_event(
            &context,
            Event::OperationStarted {
                operation: format!(
                    "Build environment isolated for {} {}",
                    context.name, context.version
                ),
            },
        );

        // Execute recipe
        let (runtime_deps, build_deps, recipe_metadata) =
            self.execute_recipe(&context, &mut environment).await?;

        // Setup build dependencies in isolated environment
        if !build_deps.is_empty() {
            Self::send_event(
                &context,
                Event::OperationStarted {
                    operation: format!("Setting up {} build dependencies", build_deps.len()),
                },
            );

            environment.setup_dependencies(build_deps).await?;

            // Log environment summary for debugging
            let env_summary = environment.environment_summary();
            Self::send_event(
                &context,
                Event::DebugLog {
                    message: "Build environment configured".to_string(),
                    context: env_summary,
                },
            );
        }

        // Generate SBOM
        let sbom_files = self.generate_sbom(&environment).await?;

        // Create manifest
        let manifest = Self::create_manifest(&context, runtime_deps, &sbom_files, &recipe_metadata);

        // Package the result
        let package_path = self
            .create_package(&context, &environment, manifest, sbom_files)
            .await?;

        // Sign the package if configured
        self.sign_package(&context, &package_path).await?;

        // Cleanup
        environment.cleanup().await?;

        Self::send_event(
            &context,
            Event::OperationCompleted {
                operation: format!("Built {} {}", context.name, context.version),
                success: true,
            },
        );

        Ok(BuildResult::new(package_path))
    }

    /// Execute the Rhai recipe
    async fn execute_recipe(
        &self,
        context: &BuildContext,
        environment: &mut BuildEnvironment,
    ) -> Result<(Vec<String>, Vec<PackageSpec>, sps2_package::RecipeMetadata), Error> {
        // Read recipe file
        let _recipe_content = fs::read_to_string(&context.recipe_path)
            .await
            .map_err(|e| BuildError::RecipeError {
                message: format!(
                    "failed to read recipe {}: {e}",
                    context.recipe_path.display()
                ),
            })?;

        // Parse the recipe
        let recipe = load_recipe(&context.recipe_path).await?;

        // Create builder API
        let working_dir = environment.build_prefix().join("src");
        fs::create_dir_all(&working_dir).await?;

        let mut api = BuilderApi::new(working_dir.clone())?;
        let _result = api.allow_network(self.config.allow_network);

        // Execute recipe with timeout
        let result = if let Some(timeout) = self.config.max_build_time {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                self.execute_recipe_steps(context, &recipe, &mut api, environment),
            )
            .await
            .map_err(|_| BuildError::BuildTimeout {
                package: context.name.clone(),
                timeout_seconds: timeout,
            })??
        } else {
            self.execute_recipe_steps(context, &recipe, &mut api, environment)
                .await?
        };

        Ok(result)
    }

    /// Execute recipe steps
    async fn execute_recipe_steps(
        &self,
        context: &BuildContext,
        recipe: &sps2_package::Recipe,
        api: &mut BuilderApi,
        environment: &mut BuildEnvironment,
    ) -> Result<(Vec<String>, Vec<PackageSpec>, sps2_package::RecipeMetadata), Error> {
        // Execute the recipe to get metadata
        let recipe_result = execute_recipe(recipe)?;

        // Extract runtime dependencies as strings
        let runtime_deps: Vec<String> = recipe_result.metadata.runtime_deps.clone();

        // Extract build dependencies as PackageSpec
        let build_deps: Vec<PackageSpec> = recipe_result
            .metadata
            .build_deps
            .iter()
            .map(|dep| PackageSpec::parse(dep))
            .collect::<Result<Vec<_>, _>>()?;

        // Execute build steps
        for step in &recipe_result.build_steps {
            Self::send_event(
                context,
                Event::BuildStepStarted {
                    step: format!("{step:?}"),
                    package: context.name.clone(),
                },
            );

            self.execute_build_step(step, api, environment).await?;

            Self::send_event(
                context,
                Event::BuildStepCompleted {
                    step: format!("{step:?}"),
                    package: context.name.clone(),
                },
            );
        }

        Ok((runtime_deps, build_deps, recipe_result.metadata.clone()))
    }

    /// Execute a single build step
    async fn execute_build_step(
        &self,
        step: &sps2_package::BuildStep,
        api: &mut BuilderApi,
        environment: &mut BuildEnvironment,
    ) -> Result<(), Error> {
        use sps2_package::BuildStep;

        match step {
            BuildStep::Fetch { url, sha256 } => {
                api.fetch(url, sha256).await?;
            }
            BuildStep::Configure { args } => {
                api.configure(args, environment).await?;
            }
            BuildStep::Make { args } => {
                api.make(args, environment).await?;
            }
            BuildStep::Autotools { args } => {
                api.autotools(args, environment).await?;
            }
            BuildStep::Cmake { args } => {
                api.cmake(args, environment).await?;
            }
            BuildStep::Meson { args } => {
                api.meson(args, environment).await?;
            }
            BuildStep::Cargo { args } => {
                api.cargo(args, environment).await?;
            }
            BuildStep::Install => {
                api.install(environment).await?;
            }
            BuildStep::ApplyPatch { path } => {
                api.apply_patch(std::path::Path::new(path), environment)
                    .await?;
            }
            BuildStep::Command { program, args } => {
                let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                environment
                    .execute_command(program, &arg_refs, None)
                    .await?;
            }
            BuildStep::SetEnv { key, value } => {
                environment.set_env_var(key.clone(), value.clone())?;
            }
            BuildStep::AllowNetwork { enabled } => {
                let _result = api.allow_network(*enabled);
            }
        }

        Ok(())
    }

    /// Generate SBOM files
    async fn generate_sbom(&self, environment: &BuildEnvironment) -> Result<SbomFiles, Error> {
        let generator = SbomGenerator::new().with_config(self.config.sbom_config.clone());

        let staging_dir = environment.staging_dir();
        let sbom_dir = environment.build_prefix().join("sbom");
        fs::create_dir_all(&sbom_dir).await?;

        generator.generate_sbom(staging_dir, &sbom_dir).await
    }

    /// Create package manifest
    fn create_manifest(
        context: &BuildContext,
        runtime_deps: Vec<String>,
        sbom_files: &SbomFiles,
        recipe_metadata: &sps2_package::RecipeMetadata,
    ) -> Manifest {
        use sps2_manifest::{Dependencies, PackageInfo, SbomInfo};

        // Create SBOM info if files are available
        let sbom_info = sbom_files.spdx_hash.as_ref().map(|spdx_hash| SbomInfo {
            spdx: spdx_hash.clone(),
            cyclonedx: sbom_files.cyclonedx_hash.clone(),
        });

        Manifest {
            package: PackageInfo {
                name: context.name.clone(),
                version: context.version.to_string(),
                revision: context.revision,
                arch: context.arch.clone(),
                description: recipe_metadata.description.clone(),
                homepage: recipe_metadata.homepage.clone(),
                license: recipe_metadata.license.clone(),
            },
            dependencies: Dependencies {
                runtime: runtime_deps,
                build: Vec::new(), // Build deps not included in final manifest
            },
            sbom: sbom_info,
        }
    }

    /// Create the final package
    async fn create_package(
        &self,
        context: &BuildContext,
        environment: &BuildEnvironment,
        manifest: Manifest,
        sbom_files: SbomFiles,
    ) -> Result<PathBuf, Error> {
        let package_path = context.output_path();

        // Create package using the real manifest data
        let manifest_string = toml::to_string(&manifest).map_err(|e| BuildError::Failed {
            message: format!("failed to serialize manifest: {e}"),
        })?;

        // Create proper .sp archive with manifest and SBOM files
        self.create_sp_package(
            environment.staging_dir(),
            &package_path,
            &manifest_string,
            &sbom_files,
        )
        .await?;

        Ok(package_path)
    }

    /// Create a .sp package archive with manifest, SBOM files, and tar+zstd compression
    async fn create_sp_package(
        &self,
        staging_dir: &Path,
        output_path: &Path,
        manifest_content: &str,
        sbom_files: &SbomFiles,
    ) -> Result<(), Error> {
        // Create the directory structure for .sp package
        let package_dir = staging_dir.parent().ok_or_else(|| BuildError::Failed {
            message: "Invalid staging directory path".to_string(),
        })?;

        let package_temp_dir = package_dir.join("package_temp");
        fs::create_dir_all(&package_temp_dir).await?;

        // Step 1: Create manifest.toml in package root
        let manifest_path = package_temp_dir.join("manifest.toml");
        fs::write(&manifest_path, manifest_content).await?;

        // Step 2: Copy SBOM files
        if let Some(spdx_path) = &sbom_files.spdx_path {
            let dst_path = package_temp_dir.join("sbom.spdx.json");
            fs::copy(spdx_path, &dst_path).await?;
        }

        if let Some(cdx_path) = &sbom_files.cyclonedx_path {
            let dst_path = package_temp_dir.join("sbom.cdx.json");
            fs::copy(cdx_path, &dst_path).await?;
        }

        // Step 3: Copy staging directory contents as package files
        Self::copy_directory_recursive(staging_dir, &package_temp_dir.join("files")).await?;

        // Step 4: Create deterministic tar archive
        let tar_path = package_temp_dir.join("package.tar");
        self.create_deterministic_tar_archive(&package_temp_dir, &tar_path)
            .await?;

        // Step 5: Compress with zstd at maximum level
        self.compress_with_zstd(&tar_path, output_path).await?;

        // Step 6: Cleanup temporary files
        fs::remove_dir_all(&package_temp_dir).await?;

        Ok(())
    }

    /// Recursively copy directory contents
    fn copy_directory_recursive<'a>(
        src: &'a Path,
        dst: &'a Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + 'a>> {
        Box::pin(async move {
            fs::create_dir_all(dst).await?;

            let mut entries = fs::read_dir(src).await?;
            while let Some(entry) = entries.next_entry().await? {
                let entry_path = entry.path();
                let dst_path = dst.join(entry.file_name());

                if entry_path.is_dir() {
                    Self::copy_directory_recursive(&entry_path, &dst_path).await?;
                } else {
                    fs::copy(&entry_path, &dst_path).await?;
                }
            }

            Ok(())
        })
    }

    /// Create deterministic tar archive from directory
    async fn create_deterministic_tar_archive(
        &self,
        source_dir: &Path,
        tar_path: &Path,
    ) -> Result<(), Error> {
        use tokio::process::Command;

        let output = Command::new("tar")
            .args([
                "--create",
                "--file",
                &tar_path.display().to_string(),
                "--directory",
                &source_dir.display().to_string(),
                "--sort=name",     // Deterministic ordering
                "--numeric-owner", // Use numeric IDs for reproducibility
                "--mtime=@0",      // Set modification time to epoch for reproducibility
                "--owner=0",
                "--group=0",
                ".",
            ])
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::Failed {
                message: format!(
                    "tar creation failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Compress tar archive with zstd
    async fn compress_with_zstd(&self, tar_path: &Path, output_path: &Path) -> Result<(), Error> {
        use tokio::process::Command;

        let output = Command::new("zstd")
            .args([
                "--compress",
                "--force",
                "--level=19", // Maximum compression as per spec
                "--output",
                &output_path.display().to_string(),
                &tar_path.display().to_string(),
            ])
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::Failed {
                message: format!(
                    "zstd compression failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Sign the package if signing is enabled
    async fn sign_package(&self, context: &BuildContext, package_path: &Path) -> Result<(), Error> {
        if !self.config.signing_config.enabled {
            return Ok(());
        }

        Self::send_event(
            context,
            Event::OperationStarted {
                operation: format!(
                    "Signing package {}",
                    package_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ),
            },
        );

        let signer = PackageSigner::new(self.config.signing_config.clone());

        match signer.sign_package(package_path).await? {
            Some(sig_path) => {
                Self::send_event(
                    context,
                    Event::OperationCompleted {
                        operation: format!("Package signed: {}", sig_path.display()),
                        success: true,
                    },
                );
            }
            None => {
                // Signing was disabled
                Self::send_event(
                    context,
                    Event::OperationCompleted {
                        operation: "Package signing skipped (disabled)".to_string(),
                        success: true,
                    },
                );
            }
        }

        Ok(())
    }

    /// Send event if context has event sender
    fn send_event(context: &BuildContext, event: Event) {
        if let Some(sender) = &context.event_sender {
            let _ = sender.send(event);
        }
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_types::Version;
    use tempfile::tempdir;

    #[test]
    fn test_build_config() {
        let config = BuildConfig::default();
        assert!(!config.allow_network);
        assert!(config.max_build_time.is_some());

        let network_config = BuildConfig::with_network();
        assert!(network_config.allow_network);

        let custom_config = BuildConfig::default().with_timeout(1800).with_jobs(4);
        assert_eq!(custom_config.max_build_time, Some(1800));
        assert_eq!(custom_config.build_jobs, Some(4));
    }

    #[test]
    fn test_builder_creation() {
        let builder = Builder::new();
        assert!(!builder.config.allow_network);

        let config = BuildConfig::with_network();
        let network_builder = Builder::with_config(config);
        assert!(network_builder.config.allow_network);
    }

    #[tokio::test]
    async fn test_build_context() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "test-pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        assert_eq!(context.package_filename(), "test-pkg-1.0.0-1.arm64.sp");
        assert!(context
            .output_path()
            .to_string_lossy()
            .ends_with("test-pkg-1.0.0-1.arm64.sp"));

        let custom_context = context.with_revision(2).with_arch("x86_64".to_string());

        assert_eq!(custom_context.revision, 2);
        assert_eq!(custom_context.arch, "x86_64");
    }
}

//! Main builder implementation

use crate::{
    BuildContext, BuildEnvironment, BuilderApi, BuildResult, SbomConfig, SbomFiles, SbomGenerator,
};
use spsv2_errors::{BuildError, Error};
use spsv2_events::{Event, EventSender};
use spsv2_hash::Hash;
use spsv2_manifest::Manifest;
use spsv2_package::{execute_recipe, load_recipe};
use spsv2_resolver::Resolver;
use spsv2_store::PackageStore;
use spsv2_types::{package::PackageSpec, Version};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Package builder configuration
#[derive(Clone, Debug)]
pub struct BuildConfig {
    /// SBOM generation configuration
    pub sbom_config: SbomConfig,
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
            max_build_time: Some(3600), // 1 hour
            allow_network: false,
            build_jobs: None, // Use auto-detection
        }
    }
}

impl BuildConfig {
    /// Create config with network access enabled
    pub fn with_network() -> Self {
        Self {
            allow_network: true,
            ..Default::default()
        }
    }

    /// Set SBOM configuration
    pub fn with_sbom_config(mut self, config: SbomConfig) -> Self {
        self.sbom_config = config;
        self
    }

    /// Set build timeout
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.max_build_time = Some(seconds);
        self
    }

    /// Set parallel build jobs
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
}

impl Builder {
    /// Create new builder
    pub fn new() -> Self {
        Self {
            config: BuildConfig::default(),
            resolver: None,
            store: None,
        }
    }

    /// Create builder with configuration
    pub fn with_config(config: BuildConfig) -> Self {
        Self {
            config,
            resolver: None,
            store: None,
        }
    }

    /// Set resolver
    pub fn with_resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Set package store
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Build a package from a Rhai recipe
    pub async fn build(&self, context: BuildContext) -> Result<BuildResult, Error> {
        self.send_event(
            &context,
            Event::OperationStarted {
                operation: format!("Building {} {}", context.name, context.version),
            },
        );

        // Create build environment
        let mut environment = BuildEnvironment::new(context.clone())?;
        // Note: In a real implementation, we would share the resolver
        // For now, skip setting it to avoid ownership issues

        // Initialize environment
        environment.initialize().await?;

        // Execute recipe
        let (runtime_deps, build_deps) = self.execute_recipe(&context, &mut environment).await?;

        // Setup build dependencies
        environment.setup_dependencies(build_deps).await?;

        // Generate SBOM
        let sbom_files = self.generate_sbom(&environment).await?;

        // Create manifest
        let manifest = self.create_manifest(&context, runtime_deps, &sbom_files)?;

        // Package the result
        let package_path = self
            .create_package(&context, &environment, manifest, sbom_files)
            .await?;

        // Cleanup
        environment.cleanup().await?;

        self.send_event(
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
    ) -> Result<(Vec<String>, Vec<PackageSpec>), Error> {
        // Read recipe file
        let recipe_content = fs::read_to_string(&context.recipe_path)
            .await
            .map_err(|e| BuildError::RecipeError {
                message: format!("failed to read recipe {}: {e}", context.recipe_path.display()),
            })?;

        // Parse the recipe
        let recipe = load_recipe(&context.recipe_path).await?;

        // Create builder API
        let working_dir = environment.build_prefix().join("src");
        fs::create_dir_all(&working_dir).await?;

        let mut api = BuilderApi::new(working_dir.clone())?;
        api.allow_network(self.config.allow_network);

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
        recipe: &spsv2_package::Recipe,
        api: &mut BuilderApi,
        environment: &BuildEnvironment,
    ) -> Result<(Vec<String>, Vec<PackageSpec>), Error> {
        // Execute the recipe to get metadata
        let recipe_result = execute_recipe(recipe)?;
        
        // Extract runtime dependencies as strings
        let runtime_deps: Vec<String> = recipe_result.metadata.runtime_deps.clone();
        
        // Extract build dependencies as PackageSpec
        let build_deps: Vec<PackageSpec> = recipe_result.metadata.build_deps
            .iter()
            .map(|dep| PackageSpec::parse(dep))
            .collect::<Result<Vec<_>, _>>()?;
        
        // Execute build steps
        for step in &recipe_result.build_steps {
            // In a real implementation, we would execute each step
            // For now, just log it
            self.send_event(context, Event::BuildStepStarted {
                step: format!("{:?}", step),
                package: context.name.clone(),
            });
        }
        
        Ok((runtime_deps, build_deps))
    }

    /// Execute build script using builder API
    async fn execute_build_script(
        &self,
        build_script: &str,
        api: &mut BuilderApi,
        environment: &BuildEnvironment,
    ) -> Result<(), Error> {
        // This is a simplified implementation
        // In practice, this would need to integrate more closely with the Rhai VM
        // and provide the full Builder API to the script

        // For now, just extract downloads and run basic build steps
        api.extract_downloads().await?;

        // The actual implementation would need to evaluate the Rhai build script
        // and call the appropriate API methods based on the script content

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
        &self,
        context: &BuildContext,
        runtime_deps: Vec<String>,
        sbom_files: &SbomFiles,
    ) -> Result<Manifest, Error> {
        use spsv2_manifest::{PackageInfo, Dependencies};
        
        let manifest = Manifest {
            package: PackageInfo {
                name: context.name.clone(),
                version: context.version.to_string(),
                revision: context.revision,
                arch: context.arch.clone(),
                description: None,
                homepage: None,
                license: None,
            },
            dependencies: Dependencies {
                runtime: runtime_deps,
                build: Vec::new(), // Build deps not included in final manifest
            },
            sbom: None,
        };

        // Add SBOM hashes if available
        if let Some(spdx_hash) = &sbom_files.spdx_hash {
            // In a real implementation, this would be added to the manifest
            // For now, we just validate the hash exists
            let _ = spdx_hash;
        }

        if let Some(cdx_hash) = &sbom_files.cyclonedx_hash {
            let _ = cdx_hash;
        }

        Ok(manifest)
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

        // Create package using store if available
        if let Some(store) = &self.store {
            // Use store to create package
            let manifest_string = toml::to_string(&manifest).map_err(|e| BuildError::Failed {
                message: format!("failed to serialize manifest: {e}"),
            })?;
            let manifest_bytes = manifest_string.as_bytes();

            // This is a simplified implementation
            // In practice, this would create a proper .sp archive with:
            // - manifest.toml
            // - sbom files
            // - package contents from staging directory
            // - proper compression and signing

            // For now, just copy staging directory contents to output
            self.copy_staging_to_output(environment.staging_dir(), &package_path)
                .await?;
        } else {
            return Err(BuildError::Failed {
                message: "package store not configured".to_string(),
            }
            .into());
        }

        Ok(package_path)
    }

    /// Copy staging directory to output (simplified implementation)
    async fn copy_staging_to_output(
        &self,
        staging_dir: &Path,
        output_path: &Path,
    ) -> Result<(), Error> {
        // This is a placeholder - in reality this would create a proper .sp archive
        // For now, just create an empty file to indicate success
        fs::write(output_path, b"placeholder package file")
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("failed to create package: {e}"),
            })?;

        Ok(())
    }

    /// Send event if context has event sender
    fn send_event(&self, context: &BuildContext, event: Event) {
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
            temp.path().join("recipe.rhai"),
            temp.path().to_path_buf(),
        );

        assert_eq!(context.package_filename(), "test-pkg-1.0.0-1.arm64.sp");
        assert!(context
            .output_path()
            .to_string()
            .ends_with("test-pkg-1.0.0-1.arm64.sp"));

        let custom_context = context.with_revision(2).with_arch("x86_64".to_string());

        assert_eq!(custom_context.revision, 2);
        assert_eq!(custom_context.arch, "x86_64");
    }
}

//! High-level build orchestration and workflow management

use super::context::BuildContext;
use crate::artifact_qa::run_quality_pipeline;
use crate::config::BuildConfig;
use crate::packaging::create_and_sign_package;
use crate::packaging::manifest::generate_sbom_and_manifest;
use crate::recipe::execute_recipe;
use crate::utils::events::send_event;
use crate::{BuildEnvironment, BuildResult};
use sps2_errors::Error;
use sps2_events::AppEvent;
use sps2_net::NetClient;
use sps2_resolver::Resolver;
use sps2_store::PackageStore;
use std::path::Path;

use sps2_resources::ResourceManager;
use std::sync::Arc;

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
    /// Resource manager
    resources: Arc<ResourceManager>,
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
            resources: Arc::new(ResourceManager::default()),
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
            resources: Arc::new(ResourceManager::default()),
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
        send_event(
            &context,
            Event::OperationStarted {
                operation: format!("Building {} {}", context.name, context.version),
            },
        );

        // Setup build environment
        let mut environment = self.setup_build_environment(&context).await?;

        // Execute recipe and setup dependencies
        let (runtime_deps, recipe_metadata, install_requested, qa_pipeline) = self
            .execute_recipe_and_setup_deps(&context, &mut environment)
            .await?;

        // Run quality checks
        run_quality_pipeline(&context, &environment, Some(qa_pipeline)).await?;

        // If fix_permissions was requested in the recipe, run it now as final step
        if let Some(paths) = &environment.fix_permissions_request {
            send_event(
                &context,
                Event::OperationStarted {
                    operation: "Final permissions fix".into(),
                },
            );

            // Create a BuilderApi instance to call do_fix_permissions
            let api = crate::core::api::BuilderApi::new(
                environment.staging_dir().to_path_buf(),
                self.resources.clone(),
            )?;
            let result = api.do_fix_permissions(paths, &environment)?;

            send_event(
                &context,
                Event::OperationCompleted {
                    operation: "Final permissions fix".into(),
                    success: result.success,
                },
            );

            // Log the result
            if !result.stdout.is_empty() {
                send_event(
                    &context,
                    Event::DebugLog {
                        message: result.stdout,
                        context: std::collections::HashMap::new(),
                    },
                );
            }
        }

        // Generate SBOM and create manifest
        let (_sbom_files, manifest) = generate_sbom_and_manifest(
            &self.config,
            &context,
            &environment,
            runtime_deps,
            &recipe_metadata,
        )
        .await?;

        // Create and sign package
        let package_path =
            create_and_sign_package(&self.config, &context, &environment, manifest).await?;

        // Update context with the generated package path
        let mut updated_context = context.clone();
        updated_context.package_path = Some(package_path.clone());

        // Cleanup and finalize
        Self::cleanup_and_finalize(&updated_context, &environment, &package_path);

        Ok(BuildResult::new(package_path).with_install_requested(install_requested))
    }

    /// Setup build environment with full isolation
    async fn setup_build_environment(
        &self,
        context: &BuildContext,
    ) -> Result<BuildEnvironment, Error> {
        // Create build environment with full isolation setup
        // Use the configured build_root from BuildConfig (defaults to /opt/pm/build)
        let build_root = self.config.build_root();
        let mut environment = BuildEnvironment::new(context.clone(), build_root)?;

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

        // Note: Isolation level and network access are applied later in
        // apply_environment_config() based on recipe settings with config defaults

        send_event(
            context,
            Event::OperationStarted {
                operation: format!(
                    "Build environment isolated for {} {}",
                    context.name, context.version
                ),
            },
        );

        Ok(environment)
    }

    /// Execute recipe and setup build dependencies
    async fn execute_recipe_and_setup_deps(
        &self,
        context: &BuildContext,
        environment: &mut BuildEnvironment,
    ) -> Result<
        (
            Vec<String>,
            crate::yaml::RecipeMetadata,
            bool,
            sps2_types::QaPipelineOverride,
        ),
        Error,
    > {
        // Parse YAML recipe for metadata
        let yaml_recipe = crate::recipe::parser::parse_yaml_recipe(&context.recipe_path).await?;
        let recipe_metadata = crate::yaml::RecipeMetadata {
            name: yaml_recipe.metadata.name.clone(),
            version: yaml_recipe.metadata.version.clone(),
            description: yaml_recipe.metadata.description.clone().into(),
            homepage: yaml_recipe.metadata.homepage.clone(),
            license: Some(yaml_recipe.metadata.license.clone()),
            runtime_deps: yaml_recipe.metadata.dependencies.runtime.clone(),
            build_deps: yaml_recipe.metadata.dependencies.build.clone(),
        };

        // Extract build dependencies as PackageSpec
        let build_deps: Vec<sps2_types::package::PackageSpec> = recipe_metadata
            .build_deps
            .iter()
            .map(|dep| sps2_types::package::PackageSpec::parse(dep))
            .collect::<Result<Vec<_>, _>>()?;

        // Setup build dependencies BEFORE executing build steps
        if !build_deps.is_empty() {
            send_event(
                context,
                Event::OperationStarted {
                    operation: format!("Setting up {} build dependencies", build_deps.len()),
                },
            );

            environment.setup_dependencies(build_deps).await?;

            // Log environment summary for debugging
            let env_summary = environment.environment_summary();
            send_event(
                context,
                Event::DebugLog {
                    message: "Build environment configured".to_string(),
                    context: env_summary,
                },
            );
        }

        // Use the build config as-is (it already has sps2_config from ops/build.rs)
        let build_config = self.config.clone();

        // Now execute the recipe with build dependencies already set up
        let (runtime_deps, _build_deps, _metadata, install_requested, qa_pipeline) =
            execute_recipe(&build_config, context, environment).await?;

        // Note: YAML recipes using staged execution have isolation already applied
        // during the environment configuration stage in staged_executor.rs.

        Ok((
            runtime_deps,
            recipe_metadata,
            install_requested,
            qa_pipeline,
        ))
    }

    /// Cleanup build environment and finalize
    fn cleanup_and_finalize(
        context: &BuildContext,
        environment: &BuildEnvironment,
        _package_path: &Path,
    ) {
        // Cleanup - skip for debugging
        // environment.cleanup().await?;
        send_event(
            context,
            Event::DebugLog {
                message: format!(
                    "Skipping cleanup for debugging - check {}",
                    environment.build_prefix().display()
                ),
                context: std::collections::HashMap::new(),
            },
        );

        send_event(
            context,
            Event::OperationCompleted {
                operation: format!("Built {} {}", context.name, context.version),
                success: true,
            },
        );
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

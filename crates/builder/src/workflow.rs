//! High-level build orchestration and workflow management

use crate::events::send_event;
use crate::manifest::generate_sbom_and_manifest;
use crate::packaging::create_and_sign_package;
use crate::quality::run_quality_checks;
use crate::recipe::execute_recipe;
use crate::{BuildConfig, BuildContext, BuildEnvironment, BuildResult};
use sps2_errors::Error;
use sps2_events::Event;
use sps2_net::NetClient;
use sps2_resolver::Resolver;
use sps2_store::PackageStore;
use std::path::Path;

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
        send_event(
            &context,
            Event::OperationStarted {
                operation: format!("Building {} {}", context.name, context.version),
            },
        );

        // Setup build environment
        let mut environment = self.setup_build_environment(&context).await?;

        // Execute recipe and setup dependencies
        let (runtime_deps, recipe_metadata, install_requested) = self
            .execute_recipe_and_setup_deps(&context, &mut environment)
            .await?;

        // Run quality checks
        run_quality_checks(&context, &environment).await?;

        // Generate SBOM and create manifest
        let (sbom_files, manifest) = generate_sbom_and_manifest(
            &self.config,
            &context,
            &environment,
            runtime_deps,
            &recipe_metadata,
        )
        .await?;

        // Create and sign package
        let package_path =
            create_and_sign_package(&self.config, &context, &environment, manifest, sbom_files)
                .await?;

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
        let build_root = self.config.build_root.as_ref()
            .expect("build_root should have a default value");
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

        // Verify isolation is properly set up
        environment.verify_isolation()?;

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
    ) -> Result<(Vec<String>, sps2_package::RecipeMetadata, bool), Error> {
        // Execute recipe
        let (runtime_deps, build_deps, recipe_metadata, install_requested) =
            execute_recipe(&self.config, context, environment).await?;

        // Setup build dependencies in isolated environment
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

        Ok((runtime_deps, recipe_metadata, install_requested))
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

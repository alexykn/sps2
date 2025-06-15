//! High-level installation operations

use crate::{
    validate_sp_file, AtomicInstaller, ExecutionContext, InstallContext, InstallResult,
    ParallelExecutor, UninstallContext, UpdateContext,
};
use sps2_errors::{Error, InstallError};
use sps2_events::Event;
use sps2_resolver::{ResolutionContext, Resolver};
use sps2_state::StateManager;
use sps2_store::PackageStore;
use sps2_types::PackageSpec;
// HashMap import removed as it's not used in this module

/// Install operation
pub struct InstallOperation {
    /// Resolver for dependencies
    resolver: Resolver,
    /// State manager
    state_manager: StateManager,
    /// Package store
    store: PackageStore,
    /// Parallel executor
    executor: ParallelExecutor,
}

impl InstallOperation {
    /// Create new install operation
    ///
    /// # Errors
    ///
    /// Returns an error if parallel executor initialization fails.
    pub fn new(
        resolver: Resolver,
        state_manager: StateManager,
        store: PackageStore,
    ) -> Result<Self, Error> {
        let executor = ParallelExecutor::new(store.clone(), state_manager.clone())?;

        Ok(Self {
            resolver,
            state_manager,
            store,
            executor,
        })
    }

    /// Execute installation
    ///
    /// # Errors
    ///
    /// Returns an error if dependency resolution fails, package download fails, or installation fails.
    pub async fn execute(&mut self, context: InstallContext) -> Result<InstallResult, Error> {
        Self::send_event(
            self,
            &context,
            Event::InstallStarting {
                packages: context
                    .packages
                    .iter()
                    .map(|spec| spec.name.clone())
                    .collect(),
            },
        );

        // Validate local .sp files before processing
        self.validate_local_packages(&context).await?;

        // Resolve dependencies
        let resolution = self.resolve_dependencies(&context).await?;

        // Execute parallel downloads
        let exec_context = ExecutionContext::new().with_event_sender(
            context
                .event_sender
                .clone()
                .unwrap_or_else(|| tokio::sync::mpsc::unbounded_channel().0),
        );

        self.executor
            .execute_parallel(&resolution.execution_plan, &resolution.nodes, &exec_context)
            .await?;

        // Downloads now populate package_map automatically via ParallelExecutor

        // Perform atomic installation
        let mut atomic_installer =
            AtomicInstaller::new(self.state_manager.clone(), self.store.clone()).await?;

        let result = atomic_installer
            .install(&context, &resolution.nodes, None)
            .await?;

        Self::send_event(
            self,
            &context,
            Event::InstallCompleted {
                packages: result
                    .installed_packages
                    .iter()
                    .map(|id| id.name.clone())
                    .collect(),
                state_id: result.state_id,
            },
        );

        Ok(result)
    }

    /// Resolve dependencies for installation
    async fn resolve_dependencies(
        &self,
        context: &InstallContext,
    ) -> Result<sps2_resolver::ResolutionResult, Error> {
        let mut resolution_context = ResolutionContext::new();

        // Add requested packages as runtime dependencies
        for spec in &context.packages {
            resolution_context = resolution_context.add_runtime_dep(spec.clone());
        }

        // Add local files
        for path in &context.local_files {
            resolution_context = resolution_context.add_local_file(path.clone());
        }

        Self::send_event(
            self,
            context,
            Event::DependencyResolving {
                package: "multiple".to_string(),
                count: context.packages.len() + context.local_files.len(),
            },
        );

        let resolution = match self.resolver.resolve_with_sat(resolution_context).await {
            Ok(result) => result,
            Err(e) => {
                // Emit helpful error event for resolution failures
                Self::send_event(
                    self,
                    context,
                    Event::Error {
                        message: "Package resolution failed".to_string(),
                        details: Some(format!(
                            "Error: {e}. \n\nPossible reasons:\n\
                            • Package name or version typo.\n\
                            • Package not available in the current repositories.\n\
                            • Version constraints are unsatisfiable.\n\
                            \nSuggested solutions:\n\
                            • Double-check package name and version specs.\n\
                            • Run 'sps2 search <package_name>' to find available packages.\n\
                            • Run 'sps2 reposync' to update your package index."
                        )),
                    },
                );
                return Err(e);
            }
        };

        Self::send_event(
            self,
            context,
            Event::DependencyResolved {
                package: "multiple".to_string(),
                version: sps2_types::Version::new(1, 0, 0), // Placeholder version
                count: resolution.nodes.len(),
            },
        );

        Ok(resolution)
    }

    /// Validate local .sp package files
    async fn validate_local_packages(&self, context: &InstallContext) -> Result<(), Error> {
        for local_file in &context.local_files {
            // Check if file exists
            if !local_file.exists() {
                return Err(InstallError::LocalPackageNotFound {
                    path: local_file.display().to_string(),
                }
                .into());
            }

            // Validate the .sp file
            let validation_result =
                validate_sp_file(local_file, context.event_sender.as_ref()).await?;

            if !validation_result.is_valid {
                return Err(InstallError::InvalidPackageFile {
                    path: local_file.display().to_string(),
                    message: "package validation failed".to_string(),
                }
                .into());
            }

            // Send warnings as events
            if let Some(sender) = &context.event_sender {
                for warning in &validation_result.warnings {
                    let _ = sender.send(Event::Warning {
                        message: warning.clone(),
                        context: Some(format!("validating {}", local_file.display())),
                    });
                }
            }

            // Log validation success
            Self::send_event(
                self,
                context,
                Event::OperationCompleted {
                    operation: format!("Validated package {}", local_file.display()),
                    success: true,
                },
            );
        }

        Ok(())
    }

    /// Send event if context has event sender
    fn send_event(_self: &Self, context: &InstallContext, event: Event) {
        if let Some(sender) = &context.event_sender {
            let _ = sender.send(event);
        }
    }
}

/// Uninstall operation
pub struct UninstallOperation {
    /// State manager
    state_manager: StateManager,
    /// Package store
    store: PackageStore,
}

impl UninstallOperation {
    /// Create new uninstall operation
    #[must_use]
    pub fn new(state_manager: StateManager, store: PackageStore) -> Self {
        Self {
            state_manager,
            store,
        }
    }

    /// Execute uninstallation
    ///
    /// # Errors
    ///
    /// Returns an error if package removal fails or dependency checks fail.
    pub async fn execute(&mut self, context: UninstallContext) -> Result<InstallResult, Error> {
        Self::send_event(
            self,
            &context,
            Event::UninstallStarting {
                packages: context.packages.clone(),
            },
        );

        // Get currently installed packages
        let current_packages = self.state_manager.get_installed_packages().await?;

        // Find packages to remove
        let mut packages_to_remove = Vec::new();
        for package_name in &context.packages {
            if let Some(package_id) = current_packages
                .iter()
                .find(|pkg| &pkg.name == package_name)
            {
                packages_to_remove.push(package_id.clone());
            } else if !context.force {
                return Err(InstallError::PackageNotInstalled {
                    package: package_name.clone(),
                }
                .into());
            }
        }

        // Check for dependents if not forcing
        if !context.force {
            for package in &packages_to_remove {
                let package_id =
                    sps2_resolver::PackageId::new(package.name.clone(), package.version());
                let dependents = self
                    .state_manager
                    .get_package_dependents(&package_id)
                    .await?;
                if !dependents.is_empty() {
                    return Err(InstallError::PackageHasDependents {
                        package: package_id.name.clone(),
                    }
                    .into());
                }
            }
        }

        if context.dry_run {
            // Return what would be removed without actually doing it
            let mut result = InstallResult::new(uuid::Uuid::new_v4());
            for package in &packages_to_remove {
                let package_id =
                    sps2_resolver::PackageId::new(package.name.clone(), package.version());
                result.add_removed(package_id);
            }
            return Ok(result);
        }

        // Perform atomic uninstallation using AtomicInstaller
        let package_ids: Vec<sps2_resolver::PackageId> = packages_to_remove
            .iter()
            .map(|pkg| sps2_resolver::PackageId::new(pkg.name.clone(), pkg.version()))
            .collect();

        let mut atomic_installer =
            AtomicInstaller::new(self.state_manager.clone(), self.store.clone()).await?;
        let result = atomic_installer.uninstall(&package_ids, &context).await?;

        Self::send_event(
            self,
            &context,
            Event::UninstallCompleted {
                packages: result
                    .removed_packages
                    .iter()
                    .map(|id| id.name.clone())
                    .collect(),
                state_id: result.state_id,
            },
        );

        Ok(result)
    }

    /// Send event if context has event sender
    fn send_event(_self: &Self, context: &UninstallContext, event: Event) {
        if let Some(sender) = &context.event_sender {
            let _ = sender.send(event);
        }
    }
}

/// Update operation
pub struct UpdateOperation {
    /// Install operation for handling updates
    install_operation: InstallOperation,
    /// State manager
    state_manager: StateManager,
}

impl UpdateOperation {
    /// Create new update operation
    ///
    /// # Errors
    ///
    /// Returns an error if install operation initialization fails.
    pub fn new(
        resolver: Resolver,
        state_manager: StateManager,
        store: PackageStore,
    ) -> Result<Self, Error> {
        let install_operation = InstallOperation::new(resolver, state_manager.clone(), store)?;

        Ok(Self {
            install_operation,
            state_manager,
        })
    }

    /// Execute update
    ///
    /// # Errors
    ///
    /// Returns an error if package resolution fails, update conflicts occur, or installation fails.
    pub async fn execute(&mut self, context: UpdateContext) -> Result<InstallResult, Error> {
        Self::send_event(
            self,
            &context,
            Event::UpdateStarting {
                packages: if context.packages.is_empty() {
                    vec!["all".to_string()]
                } else {
                    context.packages.clone()
                },
            },
        );

        // Get currently installed packages
        let current_packages = self.state_manager.get_installed_packages().await?;

        // Determine packages to update
        let packages_to_update = if context.packages.is_empty() {
            // Update all packages
            current_packages
        } else {
            // Update specified packages
            current_packages
                .into_iter()
                .filter(|pkg| context.packages.contains(&pkg.name))
                .collect()
        };

        // Convert to package specs for installation
        let mut install_context = InstallContext::new();

        for package_id in &packages_to_update {
            let spec = if context.upgrade {
                // Upgrade mode: ignore upper bounds
                PackageSpec::parse(&format!("{}>=0.0.0", package_id.name))?
            } else {
                // Update mode: respect constraints (compatible release)
                PackageSpec::parse(&format!("{}~={}", package_id.name, package_id.version))?
            };

            install_context = install_context.add_package(spec);
        }

        install_context = install_context
            .with_force(true) // Force reinstallation for updates
            .with_dry_run(context.dry_run);

        if let Some(sender) = &context.event_sender {
            install_context = install_context.with_event_sender(sender.clone());
        }

        // Execute installation (which handles updates)
        let result = self.install_operation.execute(install_context).await?;

        Self::send_event(
            self,
            &context,
            Event::UpdateCompleted {
                packages: result
                    .updated_packages
                    .iter()
                    .map(|id| id.name.clone())
                    .collect(),
                state_id: result.state_id,
            },
        );

        Ok(result)
    }

    /// Send event if context has event sender
    fn send_event(_self: &Self, context: &UpdateContext, event: Event) {
        if let Some(sender) = &context.event_sender {
            let _ = sender.send(event);
        }
    }
}

//! High-level installation operations

use crate::SecurityPolicy;
use crate::{
    AtomicInstaller, ExecutionContext, InstallContext, InstallResult, ParallelExecutor,
    UninstallContext, UpdateContext,
};
use sps2_errors::{Error, InstallError};
use sps2_events::events::GeneralEvent;
use sps2_events::{AppEvent, EventEmitter};

use sps2_resolver::{NodeAction, ResolutionContext, ResolutionResult, Resolver};
use sps2_state::StateManager;
use sps2_store::PackageStore;
use sps2_types::PackageSpec;
use std::sync::Arc;

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
        // Create a default ResourceManager for the ParallelExecutor
        let resources = Arc::new(sps2_resources::ResourceManager::default());
        let executor = ParallelExecutor::new(store.clone(), state_manager.clone(), resources)?;

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
        // Check local .sp files exist (validation moved to AtomicInstaller)
        self.check_local_packages_exist(&context)?;

        // Check for already installed packages (handled during atomic install)

        // Resolve dependencies
        let resolution = self.resolve_dependencies(&context).await?;

        // Check for already installed packages after resolution
        self.check_already_installed_resolved(&resolution)?;

        // Execute parallel downloads
        let exec_context = ExecutionContext::new()
            .with_event_sender(
                context
                    .event_sender
                    .clone()
                    .unwrap_or_else(|| sps2_events::channel().0),
            )
            .with_security_policy(SecurityPolicy {
                verify_signatures: true, // default to verify in this path
                allow_unsigned: false,
            })
            .with_force_redownload(context.force_download);

        // Debug: Check what packages we're trying to process
        context.emit_debug(format!(
            "DEBUG: About to process {} resolved packages via ParallelExecutor",
            resolution.nodes.len()
        ));

        let prepared_packages = self
            .executor
            .execute_parallel(&resolution.execution_plan, &resolution.nodes, &exec_context)
            .await?;

        // Debug: Check what packages were prepared
        context.emit_debug(format!(
            "DEBUG: ParallelExecutor prepared {} packages",
            prepared_packages.len()
        ));

        // ParallelExecutor now returns prepared package data instead of doing database operations

        // Perform atomic installation
        let mut atomic_installer =
            AtomicInstaller::new(self.state_manager.clone(), self.store.clone());

        let result = atomic_installer
            .install(&context, &resolution.nodes, Some(&prepared_packages))
            .await?;

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

        context.emit_operation_started("Resolving dependencies");

        let resolution = match self.resolver.resolve_with_sat(resolution_context).await {
            Ok(result) => result,
            Err(e) => {
                // Emit helpful error event for resolution failures
                context.emit_error_with_details(
                    "Package resolution failed",
                    format!(
                        "Error: {e}. \n\nPossible reasons:\n\
                        • Package name or version typo.\n\
                        • Package not available in the current repositories.\n\
                        • Version constraints are unsatisfiable.\n\
                        \nSuggested solutions:\n\
                        • Double-check package name and version specs.\n\
                        • Run 'sps2 search <package_name>' to find available packages.\n\
                        • Run 'sps2 reposync' to update your package index."
                    ),
                );
                return Err(e);
            }
        };

        context.emit_operation_completed("Dependency resolution", true);

        Ok(resolution)
    }

    /// Check local .sp package files exist (validation moved to AtomicInstaller)
    fn check_local_packages_exist(&self, context: &InstallContext) -> Result<(), Error> {
        for local_file in &context.local_files {
            // Check if file exists
            if !local_file.exists() {
                return Err(InstallError::LocalPackageNotFound {
                    path: local_file.display().to_string(),
                }
                .into());
            }

            // Check file extension
            if local_file.extension().is_none_or(|ext| ext != "sp") {
                return Err(InstallError::InvalidPackageFile {
                    path: local_file.display().to_string(),
                    message: "file must have .sp extension".to_string(),
                }
                .into());
            }

            // Validation moved to AtomicInstaller where it actually happens
        }

        Ok(())
    }

    /// Check for already installed packages after resolution
    fn check_already_installed_resolved(&self, resolution: &ResolutionResult) -> Result<(), Error> {
        // Check if any resolved nodes are local (already installed)
        for node in resolution.packages_in_order() {
            if let NodeAction::Local = node.action {
                // This package is already installed, emit a warning but don't error
                // The resolver has already handled this correctly
                // Package is already installed, the resolver has handled this correctly
            }
        }
        Ok(())
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

        // Perform atomic uninstallation using AtomicInstaller
        let package_ids: Vec<sps2_resolver::PackageId> = packages_to_remove
            .iter()
            .map(|pkg| sps2_resolver::PackageId::new(pkg.name.clone(), pkg.version()))
            .collect();

        let mut atomic_installer =
            AtomicInstaller::new(self.state_manager.clone(), self.store.clone());
        let result = atomic_installer.uninstall(&package_ids, &context).await?;

        Ok(result)
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
        use std::collections::HashMap;

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

        // Check if any updates are actually needed
        if packages_to_update.is_empty() {
            // No packages to update - return early with empty result
            let result = InstallResult::new(uuid::Uuid::nil());

            return Ok(result);
        }

        // For each package, check if an update is available before proceeding
        let mut packages_needing_update = Vec::new();
        let mut packages_up_to_date = Vec::new();

        for package_id in &packages_to_update {
            let spec = if context.upgrade {
                // Upgrade mode: ignore upper bounds
                PackageSpec::parse(&format!("{}>=0.0.0", package_id.name))?
            } else {
                // Update mode: respect constraints (compatible release)
                PackageSpec::parse(&format!("{}~={}", package_id.name, package_id.version))?
            };

            // Create resolution context to check for available updates
            let mut resolution_context = ResolutionContext::new();
            resolution_context = resolution_context.add_runtime_dep(spec);

            // Resolve to see what version would be installed
            match self
                .install_operation
                .resolver
                .resolve_with_sat(resolution_context)
                .await
            {
                Ok(resolution_result) => {
                    // Check if any resolved package is newer than current
                    let mut found_update = false;

                    for (resolved_id, node) in &resolution_result.nodes {
                        if resolved_id.name == package_id.name {
                            match resolved_id.version.cmp(&package_id.version()) {
                                std::cmp::Ordering::Greater => {
                                    // Update available - add to list
                                    packages_needing_update.push(package_id.clone());
                                    found_update = true;

                                    // Emit event for available update
                                    context.emit(AppEvent::General(
                                        GeneralEvent::CheckModePreview {
                                            operation: if context.upgrade {
                                                "upgrade".to_string()
                                            } else {
                                                "update".to_string()
                                            },
                                            action: format!(
                                                "Would {} {} {} → {}",
                                                if context.upgrade { "upgrade" } else { "update" },
                                                package_id.name,
                                                package_id.version,
                                                resolved_id.version
                                            ),
                                            details: HashMap::from([
                                                (
                                                    "current_version".to_string(),
                                                    package_id.version.to_string(),
                                                ),
                                                (
                                                    "new_version".to_string(),
                                                    resolved_id.version.to_string(),
                                                ),
                                                ("change_type".to_string(), "unknown".to_string()),
                                                (
                                                    "source".to_string(),
                                                    match node.action {
                                                        sps2_resolver::NodeAction::Download => {
                                                            "repository".to_string()
                                                        }
                                                        sps2_resolver::NodeAction::Local => {
                                                            "local file".to_string()
                                                        }
                                                    },
                                                ),
                                            ]),
                                        },
                                    ));
                                    break;
                                }
                                std::cmp::Ordering::Equal => {
                                    // Already up to date - add to list
                                    packages_up_to_date.push(package_id.name.clone());
                                    found_update = true;

                                    // Emit event for up to date package
                                    context.emit(AppEvent::General(
                                        GeneralEvent::CheckModePreview {
                                            operation: if context.upgrade {
                                                "upgrade".to_string()
                                            } else {
                                                "update".to_string()
                                            },
                                            action: format!(
                                                "{}:{} is already at {} version",
                                                package_id.name,
                                                package_id.version,
                                                if context.upgrade {
                                                    "latest"
                                                } else {
                                                    "compatible"
                                                }
                                            ),
                                            details: HashMap::from([
                                                (
                                                    "version".to_string(),
                                                    package_id.version.to_string(),
                                                ),
                                                ("status".to_string(), "up_to_date".to_string()),
                                            ]),
                                        },
                                    ));
                                    break;
                                }
                                std::cmp::Ordering::Less => {
                                    // This shouldn't happen normally
                                }
                            }
                            break;
                        }
                    }

                    if !found_update {
                        // No update found, package is up to date
                        packages_up_to_date.push(package_id.name.clone());
                        context.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                            operation: if context.upgrade {
                                "upgrade".to_string()
                            } else {
                                "update".to_string()
                            },
                            action: format!(
                                "{}:{} is already at {} version",
                                package_id.name,
                                package_id.version,
                                if context.upgrade {
                                    "latest"
                                } else {
                                    "compatible"
                                }
                            ),
                            details: HashMap::from([
                                ("version".to_string(), package_id.version.to_string()),
                                ("status".to_string(), "up_to_date".to_string()),
                            ]),
                        }));
                    }
                }
                Err(_) => {
                    // Resolution failed - package might not be available in repository
                    context.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                        operation: if context.upgrade {
                            "upgrade".to_string()
                        } else {
                            "update".to_string()
                        },
                        action: format!(
                            "Cannot check {}s for {}",
                            if context.upgrade { "upgrade" } else { "update" },
                            package_id.name
                        ),
                        details: HashMap::from([
                            (
                                "current_version".to_string(),
                                package_id.version.to_string(),
                            ),
                            ("status".to_string(), "resolution_failed".to_string()),
                            (
                                "reason".to_string(),
                                "package not found in repository".to_string(),
                            ),
                        ]),
                    }));
                }
            }
        }

        // If no packages need updating, return early
        if packages_needing_update.is_empty() {
            let result = InstallResult::new(uuid::Uuid::nil());

            return Ok(result);
        }

        // Convert to package specs for installation
        let mut install_context = InstallContext::new();

        for package_id in &packages_needing_update {
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
;

        if let Some(sender) = &context.event_sender {
            install_context = install_context.with_event_sender(sender.clone());
        }

        // Execute installation (which handles updates)
        let result = self.install_operation.execute(install_context).await?;

        Ok(result)
    }
}

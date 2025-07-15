//! High-level installation operations

use crate::{
    AtomicInstaller, ExecutionContext, InstallContext, InstallResult, ParallelExecutor,
    UninstallContext, UpdateContext,
};
use sps2_errors::{Error, InstallError};
use sps2_events::Event;
use sps2_events::EventEmitter;

use sps2_resolver::{ResolutionContext, Resolver};
use sps2_state::StateManager;
use sps2_store::PackageStore;
use sps2_types::PackageSpec;
use std::sync::Arc;
// HashMap import removed as it's not used in this module

impl EventEmitter for UpdateContext {
    fn event_sender(&self) -> Option<&sps2_events::EventSender> {
        self.event_sender.as_ref()
    }
}

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
        let resources = Arc::new(crate::common::resource::ResourceManager::default());
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
        context.emit_event(Event::InstallStarting {
            packages: context.packages.iter().map(|p| p.name.clone()).collect(),
        });

        // Check local .sp files exist (validation moved to AtomicInstaller)
        self.check_local_packages_exist(&context)?;

        // Check for already installed packages
        self.check_already_installed(&context).await?;

        // Resolve dependencies
        let resolution = self.resolve_dependencies(&context).await?;

        // Execute parallel downloads
        let exec_context = ExecutionContext::new().with_event_sender(
            context
                .event_sender
                .clone()
                .unwrap_or_else(|| tokio::sync::mpsc::unbounded_channel().0),
        );

        // Debug: Check what packages we're trying to process
        context.emit_event(Event::DebugLog {
            message: format!(
                "DEBUG: About to process {} resolved packages via ParallelExecutor",
                resolution.nodes.len()
            ),
            context: std::collections::HashMap::from([(
                "packages".to_string(),
                resolution
                    .nodes
                    .keys()
                    .map(|id| format!("{}-{}", id.name, id.version))
                    .collect::<Vec<_>>()
                    .join(", "),
            )]),
        });

        let prepared_packages = self
            .executor
            .execute_parallel(&resolution.execution_plan, &resolution.nodes, &exec_context)
            .await?;

        // Debug: Check what packages were prepared
        context.emit_event(Event::DebugLog {
            message: format!(
                "DEBUG: ParallelExecutor prepared {} packages",
                prepared_packages.len()
            ),
            context: std::collections::HashMap::from([(
                "prepared_packages".to_string(),
                prepared_packages
                    .keys()
                    .map(|id| format!("{}-{}", id.name, id.version))
                    .collect::<Vec<_>>()
                    .join(", "),
            )]),
        });

        // ParallelExecutor now returns prepared package data instead of doing database operations

        // Perform atomic installation
        let mut atomic_installer =
            AtomicInstaller::new(self.state_manager.clone(), self.store.clone()).await?;

        let result = atomic_installer
            .install(&context, &resolution.nodes, Some(&prepared_packages))
            .await?;

        context.emit_event(Event::InstallCompleted {
            packages: result
                .installed_packages
                .iter()
                .map(|id| id.name.clone())
                .collect(),
            state_id: result.state_id,
        });

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

        context.emit_event(Event::DependencyResolving {
            package: "multiple".to_string(),
            count: context.packages.len() + context.local_files.len(),
        });

        let resolution = match self.resolver.resolve_with_sat(resolution_context).await {
            Ok(result) => result,
            Err(e) => {
                // Emit helpful error event for resolution failures
                context.emit_event(Event::Error {
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
                });
                return Err(e);
            }
        };

        context.emit_event(Event::DependencyResolved {
            package: "multiple".to_string(),
            version: sps2_types::Version::new(1, 0, 0), // Placeholder version
            count: resolution.nodes.len(),
        });

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

    /// Check for already installed packages
    async fn check_already_installed(&self, context: &InstallContext) -> Result<(), Error> {
        // Get currently installed packages
        let installed_packages = self.state_manager.get_installed_packages().await?;

        // Check remote package specs
        for spec in &context.packages {
            // Check if this exact version is already installed
            if let Some(installed_pkg) = installed_packages.iter().find(|pkg| pkg.name == spec.name)
            {
                if spec.version_spec.matches(&installed_pkg.version()) {
                    // Send informative event
                    context.emit_event(Event::Warning {
                        message: format!(
                            "Package {}-{} is already installed",
                            installed_pkg.name, installed_pkg.version
                        ),
                        context: Some(
                            "Skipping installation to avoid state corruption".to_string(),
                        ),
                    });

                    // For now, we'll return an error to prevent state corruption
                    // In the future, we might want to handle this more gracefully
                    return Err(InstallError::PackageAlreadyInstalled {
                        package: format!("{}-{}", installed_pkg.name, installed_pkg.version),
                    }
                    .into());
                }
            }
        }

        // Check local .sp files for already installed packages
        // Try to infer package info from filename first, fall back to manifest extraction if needed
        for local_file in &context.local_files {
            // Try to parse package info from filename (e.g., gmp-6.3.0-1.arm64.sp)
            if let Some(filename) = local_file.file_stem().and_then(|s| s.to_str()) {
                if let Some((name, version)) = Self::parse_package_filename(filename) {
                    // Check if this package name/version is already installed
                    if let Some(_installed_pkg) = installed_packages
                        .iter()
                        .find(|pkg| pkg.name == name && pkg.version == version)
                    {
                        context.emit_event(Event::Warning {
                            message: format!(
                                "Package {}-{} from {} is already installed",
                                name,
                                version,
                                local_file.display()
                            ),
                            context: Some(
                                "Skipping installation to avoid state corruption".to_string(),
                            ),
                        });

                        return Err(InstallError::PackageAlreadyInstalled {
                            package: format!("{}-{}", name, version),
                        }
                        .into());
                    }
                }
                // If filename parsing fails, we'll let AtomicInstaller handle the validation
                // and duplicate detection during the actual installation process
            }
        }

        Ok(())
    }

    /// Parse package name and version from filename (e.g., "gmp-6.3.0-1.arm64" -> ("gmp", "6.3.0"))
    fn parse_package_filename(filename: &str) -> Option<(String, String)> {
        // Expected format: {name}-{version}-{build}.{arch}
        // Example: gmp-6.3.0-1.arm64
        let parts: Vec<&str> = filename.split('-').collect();
        if parts.len() >= 3 {
            let name = parts[0].to_string();
            // Version is everything between first dash and last dash (before build number)
            let version_parts: Vec<&str> = parts[1..parts.len() - 1].to_vec();
            let version = version_parts.join("-");
            Some((name, version))
        } else {
            None
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
        context.emit_event(Event::UninstallStarting {
            packages: context.packages.clone(),
        });

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

        context.emit_event(Event::UninstallCompleted {
            packages: result
                .removed_packages
                .iter()
                .map(|id| id.name.clone())
                .collect(),
            state_id: result.state_id,
        });

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
        context.emit_event(Event::UpdateStarting {
            packages: if context.packages.is_empty() {
                vec!["all".to_string()]
            } else {
                context.packages.clone()
            },
        });

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

        context.emit_event(Event::UpdateCompleted {
            packages: result
                .updated_packages
                .iter()
                .map(|id| id.name.clone())
                .collect(),
            state_id: result.state_id,
        });

        Ok(result)
    }
}

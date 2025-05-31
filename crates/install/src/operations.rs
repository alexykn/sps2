//! High-level installation operations

use crate::{
    AtomicInstaller, ExecutionContext, InstallContext, InstallResult, ParallelExecutor,
    UninstallContext, UpdateContext,
};
use spsv2_errors::{Error, InstallError};
use spsv2_events::Event;
use spsv2_resolver::{PackageId, ResolutionContext, Resolver};
use spsv2_state::{manager::StateTransition, StateManager};
use spsv2_store::PackageStore;
use spsv2_types::PackageSpec;
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
        let executor = ParallelExecutor::new(store.clone())?;

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

        // Perform atomic installation
        let mut atomic_installer =
            AtomicInstaller::new(self.state_manager.clone(), self.store.clone());

        let result = atomic_installer
            .install(&context, &resolution.nodes)
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
    ) -> Result<spsv2_resolver::ResolutionResult, Error> {
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

        let resolution = self.resolver.resolve(resolution_context).await?;

        Self::send_event(
            self,
            context,
            Event::DependencyResolved {
                package: "multiple".to_string(),
                version: spsv2_types::Version::new(1, 0, 0), // Placeholder version
                count: resolution.nodes.len(),
            },
        );

        Ok(resolution)
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
}

impl UninstallOperation {
    /// Create new uninstall operation
    #[must_use]
    pub fn new(state_manager: StateManager, _store: PackageStore) -> Self {
        Self { state_manager }
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
                    spsv2_resolver::PackageId::new(package.name.clone(), package.version());
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
                    spsv2_resolver::PackageId::new(package.name.clone(), package.version());
                result.add_removed(package_id);
            }
            return Ok(result);
        }

        // Perform atomic uninstallation
        let package_ids: Vec<spsv2_resolver::PackageId> = packages_to_remove
            .iter()
            .map(|pkg| spsv2_resolver::PackageId::new(pkg.name.clone(), pkg.version()))
            .collect();

        let result = self.remove_packages(&package_ids, &context).await?;

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

    /// Remove packages from system using atomic state transitions
    async fn remove_packages(
        &self,
        packages: &[PackageId],
        context: &UninstallContext,
    ) -> Result<InstallResult, Error> {
        // Begin state transition
        let transition = self.state_manager.begin_transition("uninstall").await?;

        // Get the packages that will be removed (including autoremove candidates)
        let mut packages_to_remove = packages.to_vec();

        if context.autoremove {
            // Find orphaned dependencies - packages that are only depended on by packages being removed
            packages_to_remove.extend(Self::find_orphaned_dependencies(packages));
        }

        // Start database transaction
        let mut tx = self.state_manager.begin_transaction().await?;

        // Create new state record
        let new_state_id = transition.to;
        spsv2_state::queries::create_state(
            &mut tx,
            &new_state_id,
            Some(&transition.from),
            "uninstall",
        )
        .await?;

        // Copy all existing packages to new state except the ones being removed
        let current_packages =
            spsv2_state::queries::get_state_packages(&mut tx, &transition.from).await?;

        let mut result = InstallResult::new(new_state_id);

        for package in &current_packages {
            let should_remove = packages_to_remove
                .iter()
                .any(|pkg| pkg.name == package.name);

            if should_remove {
                // Mark as removed and decrement store references
                let package_id = spsv2_resolver::PackageId::new(
                    package.name.clone(),
                    spsv2_types::Version::parse(&package.version)
                        .map_err(|e| Error::internal(format!("invalid version: {e}")))?,
                );
                result.add_removed(package_id);

                // Decrement store reference for the package hash
                spsv2_state::queries::decrement_store_ref(&mut tx, &package.hash).await?;
            } else {
                // Add to new state
                spsv2_state::queries::add_package(
                    &mut tx,
                    &new_state_id,
                    &package.name,
                    &package.version,
                    &package.hash,
                    package.size,
                )
                .await?;
            }
        }

        // Remove hard links from staging directory
        self.remove_package_hardlinks(&transition, &packages_to_remove)
            .await?;

        // Commit the state transition
        self.state_manager
            .commit_transition(transition, Vec::new(), Vec::new())
            .await?;

        // Commit database transaction
        tx.commit().await?;

        Ok(result)
    }

    /// Remove hard links for packages from staging directory
    async fn remove_package_hardlinks(
        &self,
        transition: &StateTransition,
        packages: &[PackageId],
    ) -> Result<(), Error> {
        for package in packages {
            // Find and remove all hard links belonging to this package
            // This is a simplified implementation - in practice, we'd need to
            // track which files belong to which packages more precisely
            let package_files = Self::get_package_files(package);

            for file_path in package_files {
                let staging_file = transition.staging_path.join(&file_path);
                if staging_file.exists() {
                    tokio::fs::remove_file(&staging_file).await.map_err(|e| {
                        spsv2_errors::InstallError::FilesystemError {
                            operation: "remove_file".to_string(),
                            path: staging_file.display().to_string(),
                            message: e.to_string(),
                        }
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Get file paths for a package (placeholder implementation)
    fn get_package_files(_package: &PackageId) -> Vec<std::path::PathBuf> {
        // TODO: Implement actual package file tracking
        // For now return empty list
        Vec::new()
    }

    /// Find orphaned dependencies that can be auto-removed
    fn find_orphaned_dependencies(_removing_packages: &[PackageId]) -> Vec<PackageId> {
        // TODO: Implement dependency analysis
        // For now return empty list
        Vec::new()
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

#[cfg(test)]
mod tests {
    use super::*;
    use spsv2_index::{Index, IndexManager};
    use tempfile::tempdir;

    async fn create_test_setup() -> (Resolver, StateManager, PackageStore) {
        let temp = tempdir().unwrap();

        // Create index manager with empty index
        let mut index_manager = IndexManager::new(temp.path());
        let index = Index::new();
        let json = index.to_json().unwrap();
        index_manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(index_manager);
        let state_manager = StateManager::new(temp.path()).await.unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        (resolver, state_manager, store)
    }

    #[tokio::test]
    async fn test_install_operation_creation() {
        let (resolver, state_manager, store) = create_test_setup().await;

        let operation = InstallOperation::new(resolver, state_manager, store).unwrap();

        // Just verify creation succeeds
        assert_eq!(operation.executor.max_concurrency(), 4);
    }

    #[tokio::test]
    async fn test_uninstall_operation_creation() {
        let (_, state_manager, store) = create_test_setup().await;

        let _operation = UninstallOperation::new(state_manager, store);

        // Just verify creation succeeds - not much to test here
    }

    #[tokio::test]
    async fn test_update_operation_creation() {
        let (resolver, state_manager, store) = create_test_setup().await;

        let _operation = UpdateOperation::new(resolver, state_manager, store).unwrap();

        // Just verify creation succeeds
    }

    #[test]
    fn test_install_context_builder() {
        let context = InstallContext::new()
            .add_package(PackageSpec::parse("curl>=8.0.0").unwrap())
            .add_local_file("/path/to/package.sp".into())
            .with_force(true)
            .with_dry_run(true);

        assert_eq!(context.packages.len(), 1);
        assert_eq!(context.local_files.len(), 1);
        assert!(context.force);
        assert!(context.dry_run);
    }

    #[test]
    fn test_uninstall_context_builder() {
        let context = UninstallContext::new()
            .add_package("curl".to_string())
            .add_package("wget".to_string())
            .with_autoremove(true)
            .with_force(true);

        assert_eq!(context.packages.len(), 2);
        assert!(context.autoremove);
        assert!(context.force);
    }

    #[test]
    fn test_update_context_builder() {
        let context = UpdateContext::new()
            .add_package("curl".to_string())
            .with_upgrade(true)
            .with_dry_run(true);

        assert_eq!(context.packages.len(), 1);
        assert!(context.upgrade);
        assert!(context.dry_run);
    }
}

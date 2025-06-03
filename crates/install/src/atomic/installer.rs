//! Atomic installer implementation using APFS optimizations

use crate::atomic::{linking, rollback, transition::StateTransition};
use crate::{InstallContext, InstallResult, StagingManager};
use sps2_errors::{Error, InstallError};
use sps2_events::Event;
use sps2_resolver::{PackageId, ResolvedNode};
use sps2_state::{PackageRef, StateManager};
use sps2_store::PackageStore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Atomic installer using APFS optimizations
pub struct AtomicInstaller {
    /// State manager for atomic transitions
    state_manager: StateManager,
    /// Package store
    store: PackageStore,
    /// Staging manager for secure extraction
    staging_manager: StagingManager,
    /// Live prefix path
    live_path: PathBuf,
}

impl AtomicInstaller {
    /// Create new atomic installer
    ///
    /// # Errors
    ///
    /// Returns an error if staging manager initialization fails
    pub async fn new(state_manager: StateManager, store: PackageStore) -> Result<Self, Error> {
        // Derive staging base path from StateManager's state path for test isolation
        let staging_base_path = state_manager.state_path().join("staging");
        let staging_manager = StagingManager::new(store.clone(), staging_base_path).await?;
        let live_path = state_manager.live_path().to_path_buf();

        Ok(Self {
            state_manager,
            store,
            staging_manager,
            live_path,
        })
    }

    /// Perform atomic installation
    ///
    /// # Errors
    ///
    /// Returns an error if state transition fails, package installation fails,
    /// or filesystem operations fail.
    pub async fn install(
        &mut self,
        context: &InstallContext,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
    ) -> Result<InstallResult, Error> {
        // Create new state transition
        let mut transition = StateTransition::new(&self.state_manager).await?;

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::StateCreating {
                state_id: transition.staging_id,
            });
        }

        // Clone current state to staging directory
        transition.create_staging(&self.live_path)?;

        // Apply package changes to staging
        let mut result = InstallResult::new(transition.staging_id);

        for (package_id, node) in resolved_packages {
            self.install_package_to_staging(&mut transition, package_id, node, &mut result)
                .await?;
        }

        if context.dry_run {
            // Clean up staging and return result without committing
            transition.cleanup().await?;
            return Ok(result);
        }

        // Commit the state transition
        transition.commit(&self.live_path).await?;

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::StateTransition {
                from: transition.parent_id.unwrap_or_default(),
                to: transition.staging_id,
                operation: "install".to_string(),
            });
        }

        Ok(result)
    }

    /// Install a single package to staging directory
    async fn install_package_to_staging(
        &self,
        transition: &mut StateTransition,
        package_id: &PackageId,
        node: &ResolvedNode,
        result: &mut InstallResult,
    ) -> Result<(), Error> {
        match &node.action {
            sps2_resolver::NodeAction::Download => {
                // Package should already be in store from download phase
                let store_path = self
                    .store
                    .get_package_path(&package_id.name, &package_id.version)?;
                self.link_package_to_staging(transition, &store_path, package_id)
                    .await?;
            }
            sps2_resolver::NodeAction::Local => {
                if let Some(local_path) = &node.path {
                    // Use the new staging system for local packages
                    self.install_local_package_with_staging(transition, local_path, package_id)
                        .await?;
                }
            }
        }

        result.add_installed(package_id.clone());
        Ok(())
    }

    /// Install a local package using the staging system
    async fn install_local_package_with_staging(
        &self,
        transition: &mut StateTransition,
        local_path: &Path,
        package_id: &PackageId,
    ) -> Result<(), Error> {
        // Extract to staging directory with validation
        let staging_dir = self
            .staging_manager
            .extract_to_staging(local_path, package_id, None)
            .await?;

        // Create staging guard for automatic cleanup on failure
        let mut staging_guard = crate::StagingGuard::new(staging_dir);

        // Get the validated staging directory
        let staging_dir =
            staging_guard
                .staging_dir()
                .ok_or_else(|| InstallError::AtomicOperationFailed {
                    message: "staging directory unavailable".to_string(),
                })?;

        // Add package to store from staging directory
        let _stored_package = self.store.add_package(local_path).await?;

        // Link package contents from staging to final location
        self.link_validated_staging_to_transition(transition, staging_dir, package_id)
            .await?;

        // Successfully processed - prevent cleanup
        let _staging_dir = staging_guard.take()?;

        Ok(())
    }

    /// Link validated staging directory contents to state transition
    async fn link_validated_staging_to_transition(
        &self,
        transition: &mut StateTransition,
        staging_dir: &crate::StagingDirectory,
        package_id: &PackageId,
    ) -> Result<(), Error> {
        let staging_files_path = staging_dir.files_path();
        let staging_prefix = &transition.staging_path;
        let mut file_paths = Vec::new();

        // Link files from validated staging to transition staging
        if staging_files_path.exists() {
            linking::create_hardlinks_recursive_with_tracking(
                &staging_files_path,
                staging_prefix,
                &staging_files_path,
                &mut file_paths,
            )
            .await?;
        }

        // Store package reference to be added during commit
        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: "placeholder-hash".to_string(), // TODO: Get from staging validation
            size: 0,                              // TODO: Calculate from staging
        };
        transition.package_refs.push(package_ref);

        // Store file information to be added during commit
        for (file_path, is_directory) in file_paths {
            transition.package_files.push((
                package_id.name.clone(),
                package_id.version.to_string(),
                file_path,
                is_directory,
            ));
        }

        Ok(())
    }

    /// Link package from store to staging directory
    async fn link_package_to_staging(
        &self,
        transition: &mut StateTransition,
        store_path: &Path,
        package_id: &PackageId,
    ) -> Result<(), Error> {
        // Create hard links from store to staging directory and collect file paths
        let staging_prefix = &transition.staging_path;
        let mut file_paths = Vec::new();

        // Walk through store package contents and create hard links
        linking::create_hardlinks_recursive_with_tracking(
            store_path,
            staging_prefix,
            store_path,
            &mut file_paths,
        )
        .await?;

        // Store package reference to be added during commit
        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: "placeholder-hash".to_string(), // TODO: Get from ResolvedNode
            size: 0,                              // TODO: Get from ResolvedNode
        };
        transition.package_refs.push(package_ref);

        // Store file information to be added during commit
        for (file_path, is_directory) in file_paths {
            transition.package_files.push((
                package_id.name.clone(),
                package_id.version.to_string(),
                file_path,
                is_directory,
            ));
        }

        Ok(())
    }

    /// Rollback to a previous state
    ///
    /// # Errors
    ///
    /// Returns an error if the target state doesn't exist, filesystem swap fails,
    /// or database update fails.
    pub async fn rollback(&mut self, target_state_id: Uuid) -> Result<(), Error> {
        rollback::rollback_to_state(&self.state_manager, &self.live_path, target_state_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InstallResult;
    use sps2_types::Version;

    #[test]
    fn test_install_result() {
        let state_id = Uuid::new_v4();
        let mut result = InstallResult::new(state_id);

        assert_eq!(result.total_changes(), 0);

        let package_id = PackageId::new("test-pkg".to_string(), Version::parse("1.0.0").unwrap());

        result.add_installed(package_id);
        assert_eq!(result.total_changes(), 1);
        assert_eq!(result.installed_packages.len(), 1);
    }
}

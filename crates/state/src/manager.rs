//! State manager implementation

use crate::{
    models::{Package, PackageRef, State, StoreRef},
    queries,
};
use sps2_errors::{Error, StateError};
use sps2_events::{Event, EventEmitter, EventSender, EventSenderExt};
use sps2_hash::Hash;
use sps2_root;
use sps2_types::StateId;
use sqlx::{Pool, Sqlite};
use std::path::PathBuf;
use uuid::Uuid;

/// State manager for atomic updates
#[derive(Clone)]
pub struct StateManager {
    pool: Pool<Sqlite>,
    state_path: PathBuf,
    live_path: PathBuf,
    tx: EventSender,
}

impl std::fmt::Debug for StateManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateManager")
            .field("state_path", &self.state_path)
            .field("live_path", &self.live_path)
            .finish_non_exhaustive()
    }
}

impl EventEmitter for StateManager {
    fn event_sender(&self) -> Option<&EventSender> {
        Some(&self.tx)
    }
}

impl StateManager {
    /// Create a new state manager with database setup
    ///
    /// # Errors
    ///
    /// Returns an error if database setup, migrations, or directory creation fails.
    pub async fn new(base_path: &std::path::Path) -> Result<Self, Error> {
        let db_path = base_path.join("state.sqlite");
        let state_path = base_path.join("states");
        let live_path = base_path.join("live");

        // Create database pool and run migrations
        let pool = crate::create_pool(&db_path).await?;
        crate::run_migrations(&pool).await?;

        // Check if we need to create an initial state
        Self::ensure_initial_state(&pool).await?;

        // Create event channel (events will be ignored for now)
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        Ok(Self {
            pool,
            state_path,
            live_path,
            tx,
        })
    }

    /// Create a new state manager with existing pool and event sender
    #[must_use]
    pub fn with_pool(
        pool: Pool<Sqlite>,
        state_path: PathBuf,
        live_path: PathBuf,
        tx: EventSender,
    ) -> Self {
        Self {
            pool,
            state_path,
            live_path,
            tx,
        }
    }

    /// Get the current active state
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or no active state exists.
    pub async fn get_active_state(&self) -> Result<StateId, Error> {
        let mut tx = self.pool.begin().await?;
        let state_id = queries::get_active_state(&mut tx).await?;
        tx.commit().await?;
        Ok(state_id)
    }

    /// Get the live path for this state manager
    #[must_use]
    pub fn live_path(&self) -> &std::path::Path {
        &self.live_path
    }

    /// Get the state path for this state manager
    #[must_use]
    pub fn state_path(&self) -> &std::path::Path {
        &self.state_path
    }

    /// Get all installed packages in current state
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_installed_packages(&self) -> Result<Vec<Package>, Error> {
        let mut tx = self.pool.begin().await?;
        let state_id = queries::get_active_state(&mut tx).await?;
        let packages = queries::get_state_packages(&mut tx, &state_id).await?;
        tx.commit().await?;
        Ok(packages)
    }

    /// Get all installed packages in a specific state
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_installed_packages_in_state(
        &self,
        state_id: &StateId,
    ) -> Result<Vec<Package>, Error> {
        let mut tx = self.pool.begin().await?;
        let packages = queries::get_state_packages(&mut tx, state_id).await?;
        tx.commit().await?;
        Ok(packages)
    }

    /// Begin a state transition
    ///
    /// # Errors
    ///
    /// Returns an error if database queries fail or filesystem operations fail.
    pub async fn begin_transition(&self, operation: &str) -> Result<StateTransition, Error> {
        // Get current state
        let mut tx = self.pool.begin().await?;
        let current_state = queries::get_active_state(&mut tx).await?;
        tx.commit().await?;

        // Create staging directory
        let staging_id = Uuid::new_v4();
        let staging_path = self.state_path.join(format!("staging-{staging_id}"));

        // Clone current state to staging (or create empty staging for first install)
        if sps2_root::exists(&self.live_path).await {
            self.tx.emit(Event::OperationStarted {
                operation: "Cloning current state".to_string(),
            });
            sps2_root::clone_directory(&self.live_path, &staging_path).await?;
        } else {
            self.tx.emit(Event::OperationStarted {
                operation: "Creating initial staging directory".to_string(),
            });
            sps2_root::create_dir_all(&staging_path).await?;
        }

        Ok(StateTransition {
            from: current_state,
            to: staging_id,
            staging_path,
            operation: operation.to_string(),
        })
    }

    /// Commit a state transition
    ///
    /// # Errors
    ///
    /// Returns an error if database transaction or filesystem operations fail.
    pub async fn commit_transition(
        &self,
        transition: StateTransition,
        packages_added: Vec<PackageRef>,
        packages_removed: Vec<PackageRef>,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;

        // Create new state record
        queries::create_state(
            &mut tx,
            &transition.to,
            Some(&transition.from),
            &transition.operation,
        )
        .await?;

        // Copy packages from parent state
        let parent_packages = queries::get_state_packages(&mut tx, &transition.from).await?;

        // Track packages in new state
        let mut new_packages = Vec::new();

        // Add existing packages (minus removed ones)
        for pkg in parent_packages {
            let removed = packages_removed
                .iter()
                .any(|r| r.package_id.name == pkg.name);
            if !removed {
                let _id = queries::add_package(
                    &mut tx,
                    &transition.to,
                    &pkg.name,
                    &pkg.version,
                    &pkg.hash,
                    pkg.size,
                )
                .await?;
                new_packages.push((pkg.hash.clone(), pkg.size));
            }
        }

        // Add new packages
        for pkg in &packages_added {
            let _id = queries::add_package(
                &mut tx,
                &transition.to,
                &pkg.package_id.name,
                &pkg.package_id.version.to_string(),
                &pkg.hash,
                pkg.size,
            )
            .await?;
            new_packages.push((pkg.hash.clone(), pkg.size));
        }

        // Update store reference counts
        for (hash, size) in &new_packages {
            queries::get_or_create_store_ref(&mut tx, hash, *size).await?;
            queries::increment_store_ref(&mut tx, hash).await?;
        }

        for pkg in &packages_removed {
            queries::decrement_store_ref(&mut tx, &pkg.hash).await?;
        }

        // Archive current live state before swapping
        let old_live_backup = self.state_path.join(transition.from.to_string());

        // Ensure state_path directory exists before creating backup paths
        sps2_root::create_dir_all(&self.state_path).await?;

        // Ensure parent directory of live_path exists
        if let Some(live_parent) = self.live_path.parent() {
            sps2_root::create_dir_all(live_parent).await?;
        }

        // Remove old backup if it exists
        if sps2_root::exists(&old_live_backup).await {
            sps2_root::remove_dir_all(&old_live_backup).await?;
        }

        // Move current live to backup, then staging to live
        if sps2_root::exists(&self.live_path).await {
            sps2_root::rename(&self.live_path, &old_live_backup).await?;
        }
        sps2_root::rename(&transition.staging_path, &self.live_path).await?;

        // Update active state
        queries::set_active_state(&mut tx, &transition.to).await?;

        // Commit transaction
        tx.commit().await?;

        self.tx.emit(Event::StateTransition {
            from: transition.from,
            to: transition.to,
            operation: transition.operation,
        });

        Ok(())
    }

    /// Rollback to a previous state
    ///
    /// # Errors
    ///
    /// Returns an error if database operations or filesystem operations fail.
    pub async fn rollback(&self, target_state: Option<StateId>) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;

        // Get current and target states
        let current_state = queries::get_active_state(&mut tx).await?;

        let target = if let Some(id) = target_state {
            id
        } else {
            // Get parent of current state
            let states = queries::get_all_states(&mut tx).await?;
            let current = states
                .iter()
                .find(|s| s.state_id() == current_state)
                .ok_or_else(|| StateError::StateNotFound {
                    id: current_state.to_string(),
                })?;

            current
                .parent_id
                .as_ref()
                .ok_or_else(|| StateError::RollbackFailed {
                    message: "No parent state to rollback to".to_string(),
                })?
                .parse()
                .map_err(|e| Error::internal(format!("invalid parent state ID: {e}")))?
        };

        tx.commit().await?;

        // Perform rollback
        let target_path = self.state_path.join(target.to_string());

        if !sps2_root::exists(&target_path).await {
            return Err(StateError::StateNotFound {
                id: target.to_string(),
            }
            .into());
        }

        // Atomic swap - exchange target state with live
        sps2_root::atomic_swap(&target_path, &self.live_path).await?;

        // Update database
        let mut tx = self.pool.begin().await?;
        queries::set_active_state(&mut tx, &target).await?;

        // Create rollback record
        let rollback_id = Uuid::new_v4();
        queries::create_state(
            &mut tx,
            &rollback_id,
            Some(&target),
            &format!("rollback from {current_state}"),
        )
        .await?;

        tx.commit().await?;

        self.tx.emit(Event::StateRollback {
            from: current_state,
            to: target,
        });

        Ok(())
    }

    /// Get state history
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_history(&self) -> Result<Vec<State>, Error> {
        let mut tx = self.pool.begin().await?;
        let states = queries::get_all_states(&mut tx).await?;
        tx.commit().await?;
        Ok(states)
    }

    /// Clean up old states
    ///
    /// # Errors
    ///
    /// Returns an error if database operations or filesystem cleanup fails.
    pub async fn cleanup(
        &self,
        retention_count: usize,
        _retention_days: u32,
    ) -> Result<CleanupResult, Error> {
        self.tx.emit(Event::CleanupStarting);

        let mut tx = self.pool.begin().await?;

        // Find states to remove using strict retention (keep only N newest states)
        let states_to_remove =
            queries::get_states_for_cleanup_strict(&mut tx, retention_count).await?;

        let mut space_freed = 0u64;

        // Remove state directories
        for state_id in &states_to_remove {
            let state_path = self.state_path.join(state_id);
            if sps2_root::exists(&state_path).await {
                space_freed += sps2_root::size(&state_path).await?;
                sps2_root::remove_dir_all(&state_path).await?;
            }
        }

        // Clean up orphaned staging directories (only if safe to remove)
        let mut entries = tokio::fs::read_dir(&self.state_path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str() {
                if name_str.starts_with("staging-") {
                    // Extract staging ID from directory name
                    if let Some(id_str) = name_str.strip_prefix("staging-") {
                        if let Ok(staging_id) = uuid::Uuid::parse_str(id_str) {
                            // Only remove if it's safe to do so
                            if self.can_remove_staging(&staging_id).await? {
                                let path = entry.path();
                                space_freed += sps2_root::size(&path).await?;
                                sps2_root::remove_dir_all(&path).await?;
                            }
                        }
                    }
                }
            }
        }

        // Log cleanup operation to gc_log table
        let total_items_removed = i64::try_from(states_to_remove.len())
            .map_err(|e| Error::internal(format!("items removed count overflow: {e}")))?;
        let space_freed_i64 = i64::try_from(space_freed)
            .map_err(|e| Error::internal(format!("space freed overflow: {e}")))?;
        queries::insert_gc_log(&mut tx, total_items_removed, space_freed_i64).await?;

        tx.commit().await?;

        self.tx.emit(Event::CleanupCompleted {
            states_removed: states_to_remove.len(),
            packages_removed: 0, // No packages removed in state cleanup
            duration_ms: 0,      // TODO: Add proper timing
        });

        Ok(CleanupResult {
            states_removed: states_to_remove.len(),
            space_freed,
        })
    }

    /// Get package dependents
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_package_dependents(
        &self,
        package_id: &sps2_resolver::PackageId,
    ) -> Result<Vec<String>, Error> {
        let mut tx = self.pool.begin().await?;
        let dependents = queries::get_package_dependents(&mut tx, &package_id.name).await?;
        tx.commit().await?;
        Ok(dependents)
    }

    /// Garbage collect unreferenced store items
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn gc_store(&self) -> Result<Vec<Hash>, Error> {
        let mut tx = self.pool.begin().await?;

        let unreferenced = queries::get_unreferenced_store_items(&mut tx).await?;
        let hashes: Vec<Hash> = unreferenced.iter().map(StoreRef::hash).collect();
        let hash_strings: Vec<String> = unreferenced.iter().map(|item| item.hash.clone()).collect();

        queries::delete_unreferenced_store_items(&mut tx, &hash_strings).await?;

        tx.commit().await?;

        Ok(hashes)
    }

    /// Garbage collect unreferenced store items with file removal
    ///
    /// # Errors
    ///
    /// Returns an error if database operations or file removal fails.
    pub async fn gc_store_with_removal(
        &self,
        store: &sps2_store::PackageStore,
    ) -> Result<usize, Error> {
        self.tx.emit(Event::OperationStarted {
            operation: "Garbage collection".to_string(),
        });

        let mut tx = self.pool.begin().await?;

        // Get unreferenced items
        let unreferenced = queries::get_unreferenced_store_items(&mut tx).await?;
        let hashes: Vec<Hash> = unreferenced.iter().map(StoreRef::hash).collect();
        let hash_strings: Vec<String> = unreferenced.iter().map(|item| item.hash.clone()).collect();

        let packages_removed = unreferenced.len();

        // Delete from database first
        queries::delete_unreferenced_store_items(&mut tx, &hash_strings).await?;

        // Log GC operation to gc_log table (only counting packages removed, space calculation is approximate)
        let packages_removed_i64 = i64::try_from(packages_removed)
            .map_err(|e| Error::internal(format!("packages removed count overflow: {e}")))?;
        let total_size: i64 = unreferenced.iter().map(|item| item.size).sum();
        queries::insert_gc_log(&mut tx, packages_removed_i64, total_size).await?;

        tx.commit().await?;

        // Remove files from store
        for hash in &hashes {
            if let Err(e) = store.remove_package(hash).await {
                // Log warning but continue with other packages
                self.tx.emit(Event::Warning {
                    message: format!("Failed to remove package {}: {e}", hash.to_hex()),
                    context: None,
                });
            }
        }

        self.tx.emit(Event::OperationCompleted {
            operation: "Garbage collection".to_string(),
            success: true,
        });

        Ok(packages_removed)
    }

    /// Add package reference
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn add_package_ref(&self, package_ref: &PackageRef) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;

        // Add package to the state
        queries::add_package(
            &mut tx,
            &package_ref.state_id,
            &package_ref.package_id.name,
            &package_ref.package_id.version.to_string(),
            &package_ref.hash,
            package_ref.size,
        )
        .await?;

        // Ensure store reference exists and increment it
        queries::get_or_create_store_ref(&mut tx, &package_ref.hash, package_ref.size).await?;
        queries::increment_store_ref(&mut tx, &package_ref.hash).await?;

        tx.commit().await?;
        Ok(())
    }

    /// Add package reference with venv path
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn add_package_ref_with_venv(
        &self,
        package_ref: &PackageRef,
        venv_path: Option<&str>,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;

        // Add package to the state with venv path
        queries::add_package_with_venv(
            &mut tx,
            &package_ref.state_id,
            &package_ref.package_id.name,
            &package_ref.package_id.version.to_string(),
            &package_ref.hash,
            package_ref.size,
            venv_path,
        )
        .await?;

        // Ensure store reference exists and increment it
        queries::get_or_create_store_ref(&mut tx, &package_ref.hash, package_ref.size).await?;
        queries::increment_store_ref(&mut tx, &package_ref.hash).await?;

        tx.commit().await?;
        Ok(())
    }

    /// Add package reference with an existing transaction
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn add_package_ref_with_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        package_ref: &PackageRef,
    ) -> Result<i64, Error> {
        // Add package to the state and get its ID
        let package_id = queries::add_package(
            tx,
            &package_ref.state_id,
            &package_ref.package_id.name,
            &package_ref.package_id.version.to_string(),
            &package_ref.hash,
            package_ref.size,
        )
        .await?;

        // Ensure store reference exists and increment it
        queries::get_or_create_store_ref(tx, &package_ref.hash, package_ref.size).await?;
        queries::increment_store_ref(tx, &package_ref.hash).await?;

        Ok(package_id)
    }

    /// Add package reference with venv path using an existing transaction
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn add_package_ref_with_venv_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        package_ref: &PackageRef,
        venv_path: Option<&str>,
    ) -> Result<i64, Error> {
        // Add package to the state with venv path and get its ID
        let package_id = queries::add_package_with_venv(
            tx,
            &package_ref.state_id,
            &package_ref.package_id.name,
            &package_ref.package_id.version.to_string(),
            &package_ref.hash,
            package_ref.size,
            venv_path,
        )
        .await?;

        // Ensure store reference exists and increment it
        queries::get_or_create_store_ref(tx, &package_ref.hash, package_ref.size).await?;
        queries::increment_store_ref(tx, &package_ref.hash).await?;

        Ok(package_id)
    }

    /// Get state path for a state ID
    ///
    /// # Errors
    ///
    /// Currently does not fail, but returns `Result` for API consistency.
    pub fn get_state_path(
        &self,
        state_id: sps2_types::StateId,
    ) -> Result<std::path::PathBuf, Error> {
        Ok(self.state_path.join(state_id.to_string()))
    }

    /// Set active state
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn set_active_state(&self, state_id: sps2_types::StateId) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        queries::set_active_state(&mut tx, &state_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Set active state with transaction
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn set_active_state_with_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        state_id: sps2_types::StateId,
    ) -> Result<(), Error> {
        queries::set_active_state(tx, &state_id).await?;
        Ok(())
    }

    /// Check if state exists
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn state_exists(&self, state_id: &sps2_types::StateId) -> Result<bool, Error> {
        let mut tx = self.pool.begin().await?;
        let exists = queries::state_exists(&mut tx, state_id).await?;
        tx.commit().await?;
        Ok(exists)
    }

    /// List all states
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn list_states(&self) -> Result<Vec<sps2_types::StateId>, Error> {
        let mut tx = self.pool.begin().await?;
        let states = queries::list_states(&mut tx).await?;
        tx.commit().await?;
        Ok(states)
    }

    /// List all states with full details
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn list_states_detailed(&self) -> Result<Vec<State>, Error> {
        let mut tx = self.pool.begin().await?;
        let states = queries::list_states_detailed(&mut tx).await?;
        tx.commit().await?;
        Ok(states)
    }

    /// Get packages in a state
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_state_packages(
        &self,
        state_id: &sps2_types::StateId,
    ) -> Result<Vec<String>, Error> {
        let mut tx = self.pool.begin().await?;
        let packages = queries::get_state_package_names(&mut tx, state_id).await?;
        tx.commit().await?;
        Ok(packages)
    }

    /// Clean up old states
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn cleanup_old_states(
        &self,
        keep_count: usize,
    ) -> Result<Vec<sps2_types::StateId>, Error> {
        let mut tx = self.pool.begin().await?;

        // Get states strictly by creation time, keeping only the N newest
        // This replaces the old age+retention logic with pure retention by creation time
        let states = queries::get_states_for_cleanup_strict(&mut tx, keep_count).await?;
        tx.commit().await?;

        // Convert strings to StateIds
        let state_ids = states
            .into_iter()
            .filter_map(|s| uuid::Uuid::parse_str(&s).ok())
            .collect();
        Ok(state_ids)
    }

    /// Get current state ID
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_current_state_id(&self) -> Result<sps2_types::StateId, Error> {
        self.get_active_state().await
    }

    /// Begin transaction (placeholder implementation)
    ///
    /// # Errors
    ///
    /// Returns an error if the database transaction cannot be started.
    pub async fn begin_transaction(&self) -> Result<sqlx::Transaction<'_, sqlx::Sqlite>, Error> {
        Ok(self.pool.begin().await?)
    }

    /// Get venv path for a package
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_package_venv_path(
        &self,
        package_name: &str,
        package_version: &str,
    ) -> Result<Option<String>, Error> {
        let mut tx = self.pool.begin().await?;
        let state_id = queries::get_active_state(&mut tx).await?;
        let venv_path =
            queries::get_package_venv_path(&mut tx, &state_id, package_name, package_version)
                .await?;
        tx.commit().await?;
        Ok(venv_path)
    }

    /// Get all packages with venvs in the current state
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_packages_with_venvs(&self) -> Result<Vec<(String, String, String)>, Error> {
        let mut tx = self.pool.begin().await?;
        let state_id = queries::get_active_state(&mut tx).await?;
        let packages = queries::get_packages_with_venvs(&mut tx, &state_id).await?;
        tx.commit().await?;
        Ok(packages)
    }

    /// Update venv path for a package
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn update_package_venv_path(
        &self,
        package_name: &str,
        package_version: &str,
        venv_path: Option<&str>,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        let state_id = queries::get_active_state(&mut tx).await?;
        queries::update_package_venv_path(
            &mut tx,
            &state_id,
            package_name,
            package_version,
            venv_path,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Create state with transaction
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn create_state_with_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        state_id: &sps2_types::StateId,
        parent_id: Option<&sps2_types::StateId>,
        operation: &str,
    ) -> Result<(), Error> {
        queries::create_state(tx, state_id, parent_id, operation).await
    }

    /// Get parent state ID
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn get_parent_state_id(
        &self,
        state_id: &sps2_types::StateId,
    ) -> Result<Option<sps2_types::StateId>, Error> {
        let mut tx = self.pool.begin().await?;
        let parent_id = queries::get_parent_state_id(&mut tx, state_id).await?;
        tx.commit().await?;
        Ok(parent_id)
    }

    /// Verify database consistency
    ///
    /// # Errors
    ///
    /// Returns an error if database verification fails.
    pub async fn verify_consistency(&self) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        // Basic verification - check if we can query the database
        let _active_state = queries::get_active_state(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Ensure an initial state exists, creating one if necessary
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    async fn ensure_initial_state(pool: &Pool<Sqlite>) -> Result<(), Error> {
        let mut tx = pool.begin().await?;

        // Check if any states exist
        let state_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM states")
            .fetch_one(&mut *tx)
            .await?;

        // If no states exist, create initial state
        if state_count == 0 {
            let initial_id = uuid::Uuid::new_v4();
            let now = chrono::Utc::now().timestamp();

            // Create initial state
            sqlx::query("INSERT INTO states (id, parent_id, created_at, operation, success) VALUES (?, NULL, ?, 'initial', 1)")
                .bind(initial_id.to_string())
                .bind(now)
                .execute(&mut *tx)
                .await?;

            // Set as active state
            sqlx::query("INSERT INTO active_state (id, state_id, updated_at) VALUES (1, ?, ?)")
                .bind(initial_id.to_string())
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Ensure store reference exists for a hash
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    pub async fn ensure_store_ref(&self, hash: &str, size: i64) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        queries::get_or_create_store_ref(&mut tx, hash, size).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Add a package to the package map
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    pub async fn add_package_map(
        &self,
        name: &str,
        version: &str,
        hash: &str,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        queries::add_package_map(&mut tx, name, version, hash).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Get the hash for a package name and version
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_package_hash(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<String>, Error> {
        let mut tx = self.pool.begin().await?;
        let hash = queries::get_package_hash(&mut tx, name, version).await?;
        tx.commit().await?;
        Ok(hash)
    }

    /// Remove a package from the package map
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    pub async fn remove_package_map(&self, name: &str, version: &str) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        queries::remove_package_map(&mut tx, name, version).await?;
        tx.commit().await?;
        Ok(())
    }

    // ===== JOURNAL MANAGEMENT FOR TWO-PHASE COMMIT =====
}

/// Data needed for transaction preparation
pub struct TransactionData<'a> {
    /// Package references to be added during commit
    pub package_refs: &'a [PackageRef],
    /// Package files to be added during commit (legacy)
    pub package_files: &'a [(String, String, String, bool)], // (package_name, package_version, file_path, is_directory)
    /// File references for file-level storage
    pub file_references: &'a [(i64, crate::FileReference)], // (package_id, file_reference)
    /// Pending file hashes to be converted to file references after packages are added
    pub pending_file_hashes: &'a [(sps2_resolver::PackageId, Vec<sps2_hash::FileHashResult>)],
}

impl StateManager {
    /// Returns the canonical path to the journal file
    fn journal_path(&self) -> PathBuf {
        self.state_path
            .parent()
            .expect("Base path must exist")
            .join("transaction.json")
    }

    /// Atomically writes the journal file
    ///
    /// # Errors
    ///
    /// Returns an error if file write or rename fails
    pub async fn write_journal(
        &self,
        journal: &sps2_types::state::TransactionJournal,
    ) -> Result<(), Error> {
        let content = serde_json::to_vec(journal)
            .map_err(|e| Error::internal(format!("Failed to serialize journal: {e}")))?;
        // Write to a temporary file first, then rename for atomicity
        let temp_path = self.journal_path().with_extension("json.tmp");
        tokio::fs::write(&temp_path, content).await?;
        tokio::fs::rename(&temp_path, self.journal_path()).await?;
        Ok(())
    }

    /// Reads and deserializes the journal file, if it exists
    ///
    /// # Errors
    ///
    /// Returns an error if file read or deserialization fails
    pub async fn read_journal(
        &self,
    ) -> Result<Option<sps2_types::state::TransactionJournal>, Error> {
        let path = self.journal_path();
        if !sps2_root::exists(&path).await {
            return Ok(None);
        }
        let content = tokio::fs::read(path).await?;
        Ok(Some(serde_json::from_slice(&content).map_err(|e| {
            Error::internal(format!("Failed to deserialize journal: {e}"))
        })?))
    }

    /// Deletes the journal file upon successful completion
    ///
    /// # Errors
    ///
    /// Returns an error if file deletion fails
    pub async fn clear_journal(&self) -> Result<(), Error> {
        let path = self.journal_path();
        if sps2_root::exists(&path).await {
            sps2_root::remove_file(&path).await?;
        }
        Ok(())
    }

    /// Check if a staging directory can be safely removed
    ///
    /// A staging directory can only be removed if:
    /// 1. No journal file exists, OR
    /// 2. The journal file exists but doesn't point to this staging directory
    ///
    /// # Errors
    ///
    /// Returns an error if journal file read fails
    pub async fn can_remove_staging(&self, staging_id: &uuid::Uuid) -> Result<bool, Error> {
        if let Some(journal) = self.read_journal().await? {
            // If journal exists and points to this staging directory, we cannot remove it
            if journal
                .staging_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == format!("staging-{staging_id}"))
                .unwrap_or(false)
            {
                return Ok(false);
            }
        }
        // Either no journal exists or it doesn't point to this staging directory
        Ok(true)
    }

    /// Phase 1 of 2PC: Prepare and commit database changes
    ///
    /// This method commits all database changes except the active state pointer,
    /// then writes the journal file as the commit point.
    ///
    /// # Errors
    ///
    /// Returns an error if database operations or journal write fails
    pub async fn prepare_transaction(
        &self,
        staging_id: &Uuid,
        parent_id: &Uuid,
        staging_path: &std::path::Path,
        operation: &str,
        transition_data: &TransactionData<'_>,
    ) -> Result<sps2_types::state::TransactionJournal, Error> {
        // Start DB transaction
        let mut tx = self.pool.begin().await?;

        // Write all DB changes: new state record, package refs, file lists
        // DO NOT update the `active_state` table yet
        self.create_state_with_tx(&mut tx, staging_id, Some(parent_id), operation)
            .await?;

        // Track package IDs for file hash processing
        let mut package_id_map = std::collections::HashMap::new();

        // Add all package references to the database
        for package_ref in transition_data.package_refs {
            let package_id = self.add_package_ref_with_tx(&mut tx, package_ref).await?;
            package_id_map.insert(package_ref.package_id.clone(), package_id);
        }

        // Add all stored package files to the database
        for (package_name, package_version, file_path, is_directory) in
            transition_data.package_files
        {
            queries::add_package_file(
                &mut tx,
                staging_id,
                package_name,
                package_version,
                file_path,
                *is_directory,
            )
            .await?;
        }

        // Process pending file hashes now that we have package IDs
        for (package_id, file_hashes) in transition_data.pending_file_hashes {
            if let Some(&db_package_id) = package_id_map.get(package_id) {
                for file_hash in file_hashes {
                    let relative_path = file_hash.relative_path.clone();

                    let file_ref = crate::FileReference {
                        package_id: db_package_id,
                        relative_path,
                        hash: file_hash.hash.clone(),
                        metadata: crate::FileMetadata {
                            size: file_hash.size as i64,
                            permissions: file_hash.mode.unwrap_or(0o644),
                            uid: 0,
                            gid: 0,
                            mtime: None,
                            is_executable: file_hash.mode.map(|m| m & 0o111 != 0).unwrap_or(false),
                            is_symlink: file_hash.is_symlink,
                            symlink_target: None,
                        },
                    };

                    // First add the file object if it doesn't exist
                    let _dedup_result =
                        queries::add_file_object(&mut tx, &file_ref.hash, &file_ref.metadata)
                            .await?;

                    // Then add the package file entry
                    queries::add_package_file_entry(&mut tx, db_package_id, &file_ref).await?;
                }
            }
        }

        // Add file-level data if available (for direct file references)
        for (package_id, file_ref) in transition_data.file_references {
            // First add the file object if it doesn't exist
            let _dedup_result =
                queries::add_file_object(&mut tx, &file_ref.hash, &file_ref.metadata).await?;

            // Then add the package file entry
            queries::add_package_file_entry(&mut tx, *package_id, file_ref).await?;
        }

        // Commit the DB transaction
        tx.commit().await?;

        // Create the journal file on disk. This is the "commit point" for Phase 1
        let journal = sps2_types::state::TransactionJournal {
            new_state_id: *staging_id,
            parent_state_id: *parent_id,
            staging_path: staging_path.to_path_buf(),
            phase: sps2_types::state::TransactionPhase::Prepared,
            operation: operation.to_string(),
        };
        self.write_journal(&journal).await?;

        Ok(journal)
    }

    /// Phase 2 of 2PC: Execute filesystem swap and finalize
    ///
    /// This method performs the filesystem operations and finalizes the transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if filesystem operations or database finalization fails
    pub async fn execute_filesystem_swap_and_finalize(
        &self,
        mut journal: sps2_types::state::TransactionJournal,
    ) -> Result<(), Error> {
        // Atomically swap the staging directory with the live directory
        sps2_root::atomic_swap(&journal.staging_path, &self.live_path).await?;

        // Archive the old live directory (which is now at the journal's staging path)
        let old_live_archive_path = self.state_path.join(journal.parent_state_id.to_string());
        sps2_root::rename(&journal.staging_path, &old_live_archive_path).await?;

        // Update the journal to the 'Swapped' phase. If a crash happens now,
        // recovery will know the swap is done
        journal.phase = sps2_types::state::TransactionPhase::Swapped;
        self.write_journal(&journal).await?;

        // Finalize the database by setting the new state as active
        self.finalize_db_state(journal.new_state_id).await?;

        // The transaction is fully complete. Delete the journal
        self.clear_journal().await?;

        Ok(())
    }

    /// Helper function for final DB state update, used by both commit and recovery
    ///
    /// # Errors
    ///
    /// Returns an error if database update fails
    pub async fn finalize_db_state(&self, new_active_id: Uuid) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        queries::set_active_state(&mut tx, &new_active_id).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// State transition handle
pub struct StateTransition {
    pub from: StateId,
    pub to: StateId,
    pub staging_path: PathBuf,
    pub operation: String,
}

/// Cleanup operation result
pub struct CleanupResult {
    pub states_removed: usize,
    pub space_freed: u64,
}

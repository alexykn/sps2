//! State manager implementation

use crate::{
    models::{Package, PackageRef, State, StoreRef},
    queries,
};
use sps2_errors::{Error, StateError};
use sps2_events::{Event, EventSender, EventSenderExt};
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
        tokio::fs::create_dir_all(&self.state_path).await?;

        // Ensure parent directory of live_path exists
        if let Some(live_parent) = self.live_path.parent() {
            tokio::fs::create_dir_all(live_parent).await?;
        }

        // Remove old backup if it exists
        if sps2_root::exists(&old_live_backup).await {
            sps2_root::remove_dir_all(&old_live_backup).await?;
        }

        // Move current live to backup, then staging to live
        if sps2_root::exists(&self.live_path).await {
            tokio::fs::rename(&self.live_path, &old_live_backup).await?;
        }
        tokio::fs::rename(&transition.staging_path, &self.live_path).await?;

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
        retention_days: u32,
    ) -> Result<CleanupResult, Error> {
        self.tx.emit(Event::CleanupStarting);

        let mut tx = self.pool.begin().await?;

        // Find states to remove
        let cutoff_time = chrono::Utc::now().timestamp() - (i64::from(retention_days) * 86400);
        let states_to_remove =
            queries::get_states_to_cleanup(&mut tx, retention_count, cutoff_time).await?;

        let mut space_freed = 0u64;

        // Remove state directories
        for state_id in &states_to_remove {
            let state_path = self.state_path.join(state_id);
            if sps2_root::exists(&state_path).await {
                space_freed += sps2_root::size(&state_path).await?;
                sps2_root::remove_dir_all(&state_path).await?;
            }
        }

        // Clean up orphaned staging directories
        let mut entries = tokio::fs::read_dir(&self.state_path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str() {
                if name_str.starts_with("staging-") {
                    let path = entry.path();
                    space_freed += sps2_root::size(&path).await?;
                    sps2_root::remove_dir_all(&path).await?;
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
    ) -> Result<(), Error> {
        // Add package to the state
        queries::add_package(
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

        Ok(())
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
    ) -> Result<(), Error> {
        // Add package to the state with venv path
        queries::add_package_with_venv(
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

        Ok(())
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
        let cutoff_time = chrono::Utc::now().timestamp() - (30 * 24 * 60 * 60); // 30 days ago
        let states = queries::get_states_for_cleanup(&mut tx, keep_count, cutoff_time).await?;
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

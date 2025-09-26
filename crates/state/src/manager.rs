//! State manager implementation

use crate::{
    models::{Package, PackageRef, State, StoreRef},
    queries,
};
use sps2_errors::Error;
use sps2_events::{AppEvent, CleanupSummary, EventEmitter, EventSender, GeneralEvent, StateEvent};
use sps2_hash::Hash;
use sps2_platform::filesystem_helpers as sps2_root;
use sps2_types::StateId;
use sqlx::{Pool, Sqlite};
use std::convert::TryFrom;
use std::path::PathBuf;
use std::time::Instant;
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
    // Helper: decrement all refs from parent and return a (name,version)->Package map with counters
    async fn decrement_parent_refs_and_build_map(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        parent_id: &Uuid,
    ) -> Result<
        (
            std::collections::HashMap<(String, String), crate::models::Package>,
            usize,
            usize,
        ),
        Error,
    > {
        let parent_packages = queries::get_state_packages(tx, parent_id).await?;
        let mut parent_pkg_map: std::collections::HashMap<
            (String, String),
            crate::models::Package,
        > = std::collections::HashMap::new();
        for pkg in &parent_packages {
            parent_pkg_map.insert((pkg.name.clone(), pkg.version.clone()), pkg.clone());
        }

        let mut store_dec_count: usize = 0;
        let mut file_dec_count: usize = 0;
        for pkg in &parent_packages {
            queries::decrement_store_ref(tx, &pkg.hash).await?;
            store_dec_count += 1;
            let dec =
                crate::file_queries_runtime::decrement_file_object_refs_for_package(tx, pkg.id)
                    .await?;
            file_dec_count += dec;
        }
        Ok((parent_pkg_map, store_dec_count, file_dec_count))
    }

    // Helper: add package refs and build map PackageId -> db id, returning increment count
    async fn add_package_refs_and_build_id_map(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        package_refs: &[PackageRef],
    ) -> Result<
        (
            std::collections::HashMap<sps2_resolver::PackageId, i64>,
            usize,
        ),
        Error,
    > {
        let mut package_id_map = std::collections::HashMap::new();
        let mut store_inc_count = 0usize;
        for package_ref in package_refs {
            let package_id = self.add_package_ref_with_tx(tx, package_ref).await?;
            package_id_map.insert(package_ref.package_id.clone(), package_id);
            store_inc_count += 1;
        }
        Ok((package_id_map, store_inc_count))
    }

    // Helper: process pending hashes for packages with computed file hashes
    async fn process_pending_file_hashes(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        package_id_map: &std::collections::HashMap<sps2_resolver::PackageId, i64>,
        pending: &[(sps2_resolver::PackageId, Vec<sps2_hash::FileHashResult>)],
    ) -> Result<usize, Error> {
        let mut file_inc_count = 0usize;
        for (package_id, file_hashes) in pending {
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

                    let _ =
                        queries::add_file_object(tx, &file_ref.hash, &file_ref.metadata).await?;
                    queries::add_package_file_entry(tx, db_package_id, &file_ref).await?;
                    file_inc_count += 1;
                }
            }
        }
        Ok(file_inc_count)
    }

    // Helper: process direct file references
    async fn process_file_references(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        file_references: &[(i64, crate::FileReference)],
    ) -> Result<usize, Error> {
        let mut file_inc_count = 0usize;
        for (package_id, file_ref) in file_references {
            let _ = queries::add_file_object(tx, &file_ref.hash, &file_ref.metadata).await?;
            queries::add_package_file_entry(tx, *package_id, file_ref).await?;
            file_inc_count += 1;
        }
        Ok(file_inc_count)
    }

    // Helper: backfill carry-forward packages' file entries from parent
    async fn backfill_carry_forward_files(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        parent_pkg_map: &std::collections::HashMap<(String, String), crate::models::Package>,
        package_id_map: &std::collections::HashMap<sps2_resolver::PackageId, i64>,
        transition_data: &TransactionData<'_>,
    ) -> Result<usize, Error> {
        use sps2_hash::Hash as Sps2Hash;
        let mut file_inc_count = 0usize;

        // Build set of packages that had file hashes computed in this transition
        let mut hashed_set: std::collections::HashSet<sps2_resolver::PackageId> =
            std::collections::HashSet::new();
        for (pid, _hashes) in transition_data.pending_file_hashes {
            hashed_set.insert(pid.clone());
        }

        for pkg_ref in transition_data.package_refs {
            if hashed_set.contains(&pkg_ref.package_id) {
                continue;
            }

            let Some(&new_db_pkg_id) = package_id_map.get(&pkg_ref.package_id) else {
                continue;
            };

            if let Some(parent_pkg) = parent_pkg_map.get(&(
                pkg_ref.package_id.name.clone(),
                pkg_ref.package_id.version.to_string(),
            )) {
                let parent_entries =
                    crate::file_queries_runtime::get_package_file_entries(tx, parent_pkg.id)
                        .await?;
                for entry in parent_entries {
                    let hash_hex = entry.file_hash.clone();
                    let fh = Sps2Hash::from_hex(&hash_hex).map_err(|e| {
                        Error::internal(format!("invalid file hash {hash_hex}: {e}"))
                    })?;

                    let existing = crate::file_queries_runtime::get_file_object(tx, &fh).await?;

                    if existing.is_none() {
                        let metadata = crate::FileMetadata {
                            size: 0,
                            permissions: entry.permissions as u32,
                            uid: entry.uid as u32,
                            gid: entry.gid as u32,
                            mtime: entry.mtime,
                            is_executable: false,
                            is_symlink: false,
                            symlink_target: None,
                        };
                        let _ = queries::add_file_object(tx, &fh, &metadata).await?;
                    }

                    let file_ref = crate::FileReference {
                        package_id: new_db_pkg_id,
                        relative_path: entry.relative_path.clone(),
                        hash: fh.clone(),
                        metadata: crate::FileMetadata {
                            size: existing.as_ref().map(|o| o.size).unwrap_or(0),
                            permissions: entry.permissions as u32,
                            uid: entry.uid as u32,
                            gid: entry.gid as u32,
                            mtime: entry.mtime,
                            is_executable: existing
                                .as_ref()
                                .map(|o| o.is_executable)
                                .unwrap_or(false),
                            is_symlink: existing.as_ref().map(|o| o.is_symlink).unwrap_or(false),
                            symlink_target: existing
                                .as_ref()
                                .and_then(|o| o.symlink_target.clone()),
                        },
                    };
                    let _ =
                        queries::add_file_object(tx, &file_ref.hash, &file_ref.metadata).await?;
                    queries::add_package_file_entry(tx, new_db_pkg_id, &file_ref).await?;
                    file_inc_count += 1;
                }
            }
        }

        Ok(file_inc_count)
    }

    // Helper: clamp negative refcounts to zero
    async fn clamp_negatives(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<(), Error> {
        sqlx::query("UPDATE store_refs SET ref_count = 0 WHERE ref_count < 0")
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE file_objects SET ref_count = 0 WHERE ref_count < 0")
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

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
        let (tx, _rx) = sps2_events::channel();

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
            sps2_root::clone_directory(&self.live_path, &staging_path).await?;
        } else {
            sps2_root::create_dir_all(&staging_path).await?;
        }

        Ok(StateTransition {
            from: current_state,
            to: staging_id,
            staging_path,
            operation: operation.to_string(),
        })
    }

    // Note: rollback is now implemented reconstructively in the installer crate.

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
        let mut tx = self.pool.begin().await?;

        // Determine states to prune (visibility) using policy
        let prune_by_count =
            queries::get_states_for_cleanup_strict(&mut tx, retention_count).await?;
        let mut prune_ids: std::collections::HashSet<String> = prune_by_count.into_iter().collect();
        if retention_days > 0 {
            let cutoff = chrono::Utc::now().timestamp() - i64::from(retention_days) * 86_400;
            let older = queries::get_states_older_than(&mut tx, cutoff).await?;
            for id in older {
                prune_ids.insert(id);
            }
        }
        let active = queries::get_active_state(&mut tx).await?;
        let active_str = active.to_string();
        let now_ts = chrono::Utc::now().timestamp();
        let prune_list: Vec<String> = prune_ids.into_iter().collect();
        let states_pruned =
            queries::mark_pruned_states(&mut tx, &prune_list, now_ts, &active_str).await?;

        // Legacy directories to remove (IDs beyond newest N)
        let states_to_remove =
            queries::get_states_for_cleanup_strict(&mut tx, retention_count).await?;

        let cleanup_start = Instant::now();
        let mut cleanup_summary = CleanupSummary {
            planned_states: states_to_remove.len(),
            removed_states: None,
            space_freed_bytes: None,
            duration_ms: None,
        };
        self.tx.emit(AppEvent::State(StateEvent::CleanupStarted {
            summary: cleanup_summary.clone(),
        }));

        let mut space_freed = 0u64;
        let mut removed_count: usize = 0;

        // Remove state directories
        for state_id in &states_to_remove {
            let state_path = self.state_path.join(state_id);
            if sps2_root::exists(&state_path).await {
                space_freed += sps2_root::size(&state_path).await?;
                sps2_root::remove_dir_all(&state_path).await?;
                removed_count += 1;
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
                                removed_count += 1;
                            }
                        }
                    }
                }
            }
        }

        // Log cleanup operation to gc_log table
        let total_items_removed = i64::try_from(removed_count)
            .map_err(|e| Error::internal(format!("items removed count overflow: {e}")))?;
        let space_freed_i64 = i64::try_from(space_freed)
            .map_err(|e| Error::internal(format!("space freed overflow: {e}")))?;
        queries::insert_gc_log(&mut tx, total_items_removed, space_freed_i64).await?;

        tx.commit().await?;

        cleanup_summary.removed_states = Some(removed_count);
        cleanup_summary.space_freed_bytes = Some(space_freed);
        cleanup_summary.duration_ms =
            Some(u64::try_from(cleanup_start.elapsed().as_millis()).unwrap_or(u64::MAX));

        self.tx.emit(AppEvent::State(StateEvent::CleanupCompleted {
            summary: cleanup_summary,
        }));

        Ok(CleanupResult {
            states_pruned,
            states_removed: removed_count,
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
        let mut tx = self.pool.begin().await?;

        // Get unreferenced items
        let unreferenced = queries::get_unreferenced_store_items(&mut tx).await?;
        let hashes: Vec<Hash> = unreferenced.iter().map(StoreRef::hash).collect();
        let hash_strings: Vec<String> = unreferenced.iter().map(|item| item.hash.clone()).collect();

        let packages_removed = unreferenced.len();
        let cleanup_start = Instant::now();
        let mut cleanup_summary = CleanupSummary {
            planned_states: packages_removed,
            removed_states: None,
            space_freed_bytes: None,
            duration_ms: None,
        };
        self.tx.emit(AppEvent::State(StateEvent::CleanupStarted {
            summary: cleanup_summary.clone(),
        }));

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
                self.tx.emit(AppEvent::General(GeneralEvent::Warning {
                    message: format!("Failed to remove package {}: {e}", hash.to_hex()),
                    context: None,
                }));
            }
        }

        let space_freed_bytes: u64 = unreferenced
            .iter()
            .map(|item| u64::try_from(item.size).unwrap_or(0))
            .sum();

        cleanup_summary.removed_states = Some(packages_removed);
        cleanup_summary.space_freed_bytes = Some(space_freed_bytes);
        cleanup_summary.duration_ms =
            Some(u64::try_from(cleanup_start.elapsed().as_millis()).unwrap_or(u64::MAX));

        self.tx.emit(AppEvent::State(StateEvent::CleanupCompleted {
            summary: cleanup_summary,
        }));

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

    /// Unprune a state so it appears again in base history
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn unprune_state(&self, state_id: &sps2_types::StateId) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        let id_str = state_id.to_string();
        queries::unprune_state(&mut tx, &id_str).await?;
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

    /// Synchronize `store_refs` and `file_objects` refcounts to match a specific state exactly
    ///
    /// This sets refcounts to the exact desired values for the given state:
    /// - For hashes present in the state: set to the number of references in that state
    /// - For hashes not present in the state: set to 0
    ///
    /// Returns (`store_updates`, `file_updates`).
    ///
    /// # Errors
    ///
    /// Returns an error if any database read or write fails while computing or applying refcounts.
    pub async fn sync_refcounts_to_state(
        &self,
        state_id: &sps2_types::StateId,
    ) -> Result<(usize, usize), Error> {
        use std::collections::HashMap;

        let mut tx = self.pool.begin().await?;

        // Build desired package-level counts by hash for the target state
        let packages = queries::get_state_packages(&mut tx, state_id).await?;
        let mut store_counts: HashMap<String, (i64 /*count*/, i64 /*size*/)> = HashMap::new();
        for p in &packages {
            store_counts
                .entry(p.hash.clone())
                .and_modify(|e| e.0 += 1)
                .or_insert((1, p.size));
        }

        // Ensure store_ref rows exist for desired hashes
        for (hash, (_cnt, size)) in &store_counts {
            queries::get_or_create_store_ref(&mut tx, hash, *size).await?;
        }

        // Set store refcounts to exact values (others -> 0)
        let rows = queries::get_all_store_refs(&mut tx).await?;
        let mut store_updates = 0usize;
        for r in rows {
            let desired = store_counts.get(&r.hash).map(|(c, _)| *c).unwrap_or(0);
            if r.ref_count != desired {
                let updated =
                    crate::queries_runtime::set_store_ref_count(&mut tx, &r.hash, desired).await?;
                if updated > 0 {
                    store_updates += 1;
                }
            }
        }

        // Build desired file-level counts by file_hash for the target state
        let mut file_counts: HashMap<String, i64> = HashMap::new();
        for p in &packages {
            let entries =
                crate::file_queries_runtime::get_package_file_entries(&mut tx, p.id).await?;
            for e in entries {
                file_counts
                    .entry(e.file_hash)
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
            }
        }

        // Set file object refcounts to exact values (others -> 0)
        let all_files = crate::file_queries_runtime::get_all_file_objects(&mut tx).await?;
        let mut file_updates = 0usize;
        for fo in all_files {
            let desired = file_counts.get(&fo.hash).copied().unwrap_or(0);
            if fo.ref_count != desired {
                let updated = crate::file_queries_runtime::set_file_object_ref_count(
                    &mut tx, &fo.hash, desired,
                )
                .await?;
                if updated > 0 {
                    file_updates += 1;
                }
            }
        }

        tx.commit().await?;
        Ok((store_updates, file_updates))
    }

    // ===== JOURNAL MANAGEMENT FOR TWO-PHASE COMMIT =====
}

/// Data needed for transaction preparation
pub struct TransactionData<'a> {
    /// Package references to be added during commit
    pub package_refs: &'a [PackageRef],
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
        // Active-state refcount semantics: decrement all refs from parent, then increment for new state
        let (parent_pkg_map, store_dec_count, file_dec_count) = self
            .decrement_parent_refs_and_build_map(&mut tx, parent_id)
            .await?;

        // Add all package references and build map of db IDs
        let (package_id_map, store_inc_count) = self
            .add_package_refs_and_build_id_map(&mut tx, transition_data.package_refs)
            .await?;

        // Add file-level entries for newly hashed packages
        let mut file_inc_count = self
            .process_pending_file_hashes(
                &mut tx,
                &package_id_map,
                transition_data.pending_file_hashes,
            )
            .await?;

        // Add file-level entries for direct file references, if any
        file_inc_count += self
            .process_file_references(&mut tx, transition_data.file_references)
            .await?;

        // Backfill file entries for carry-forward packages that didn't compute hashes
        file_inc_count += self
            .backfill_carry_forward_files(
                &mut tx,
                &parent_pkg_map,
                &package_id_map,
                transition_data,
            )
            .await?;

        // Clamp negatives to zero defensively
        self.clamp_negatives(&mut tx).await?;

        // Commit the DB transaction
        tx.commit().await?;

        // Emit debug summary
        self.emit_debug(format!(
            "2PC refcounts: store +{store_inc_count} -{store_dec_count}, files +{file_inc_count} -{file_dec_count}"
        ));

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
        // New default behavior: atomically rename staging to live (no archive kept)
        // Ensure parent directory of live_path exists
        if let Some(live_parent) = self.live_path.parent() {
            sps2_root::create_dir_all(live_parent).await?;
        }
        sps2_root::atomic_rename(&journal.staging_path, &self.live_path).await?;

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
    pub states_pruned: usize,
    pub states_removed: usize,
    pub space_freed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{file_queries_runtime as fq, queries};
    use tempfile::TempDir;

    async fn mk_state() -> (TempDir, StateManager) {
        let td = TempDir::new().expect("tempdir");
        let mgr = StateManager::new(td.path()).await.expect("state new");
        (td, mgr)
    }

    async fn store_ref_count(state: &StateManager, hash: &str) -> i64 {
        let mut tx = state.begin_transaction().await.expect("tx");
        let rows = queries::get_all_store_refs(&mut tx).await.expect("refs");
        tx.commit().await.expect("commit");
        rows.into_iter()
            .find(|r| r.hash == hash)
            .map(|r| r.ref_count)
            .unwrap_or(0)
    }

    async fn file_obj_ref_count(state: &StateManager, hash: &str) -> i64 {
        let mut tx = state.begin_transaction().await.expect("tx");
        let h = sps2_hash::Hash::from_hex(hash).expect("hash");
        let row = fq::get_file_object(&mut tx, &h).await.expect("get");
        tx.commit().await.expect("commit");
        row.map(|o| o.ref_count).unwrap_or(0)
    }

    async fn seed_parent_with_pkg(
        state: &StateManager,
        name: &str,
        version: &str,
        pkg_hash_hex: &str,
        files: &[(&str, i64)],
    ) {
        let mut tx = state.begin_transaction().await.expect("tx");
        let sid = queries::get_active_state(&mut tx).await.expect("sid");
        let pkg_id = queries::add_package(&mut tx, &sid, name, version, pkg_hash_hex, 1)
            .await
            .expect("add pkg");

        for (rel, size) in files {
            let fh = sps2_hash::Hash::from_data(rel.as_bytes());
            let meta = crate::FileMetadata::regular_file(*size, 0o644);
            let _ = fq::add_file_object(&mut tx, &fh, &meta)
                .await
                .expect("add fo");
            let fr = crate::FileReference {
                package_id: pkg_id,
                relative_path: (*rel).to_string(),
                hash: fh,
                metadata: meta,
            };
            let _ = fq::add_package_file_entry(&mut tx, pkg_id, &fr)
                .await
                .expect("pfe");
        }
        tx.commit().await.expect("commit");
    }

    #[tokio::test]
    async fn t_2pc_carry_forward_net_zero() {
        let (_td, state) = mk_state().await;
        let parent_id = state.get_active_state().await.expect("parent");
        let pkg_hash = sps2_hash::Hash::from_data(b"pkg-A").to_hex();
        seed_parent_with_pkg(
            &state,
            "A",
            "1.0.0",
            &pkg_hash,
            &[("bin/a", 3), ("bin/b", 4)],
        )
        .await;

        // build transition data with carry-forward only
        let staging_id = uuid::Uuid::new_v4();
        let pid = sps2_resolver::PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        let pref = PackageRef {
            state_id: staging_id,
            package_id: pid,
            hash: pkg_hash.clone(),
            size: 1,
        };
        let td = TransactionData {
            package_refs: &[pref],
            file_references: &[],
            pending_file_hashes: &[],
        };

        let staging_path = state.state_path().join(format!("staging-{staging_id}"));
        let _journal = state
            .prepare_transaction(&staging_id, &parent_id, &staging_path, "test-carry", &td)
            .await
            .expect("prepare");

        // store ref should be 1 for A
        assert_eq!(store_ref_count(&state, &pkg_hash).await, 1);

        // file refs should be 1 for both files
        let f1 = sps2_hash::Hash::from_data(b"bin/a").to_hex();
        let f2 = sps2_hash::Hash::from_data(b"bin/b").to_hex();
        assert_eq!(file_obj_ref_count(&state, &f1).await, 1);
        assert_eq!(file_obj_ref_count(&state, &f2).await, 1);
    }

    #[tokio::test]
    async fn t_2pc_install_increments() {
        let (_td, state) = mk_state().await;
        let parent_id = state.get_active_state().await.expect("parent");
        let pkg_hash = sps2_hash::Hash::from_data(b"pkg-B").to_hex();
        let staging_id = uuid::Uuid::new_v4();
        let pid = sps2_resolver::PackageId::new(
            "B".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        let pref = PackageRef {
            state_id: staging_id,
            package_id: pid,
            hash: pkg_hash.clone(),
            size: 1,
        };
        let file_hashes = vec![
            sps2_hash::FileHashResult {
                relative_path: "bin/x".to_string(),
                size: 1,
                mode: Some(0o755),
                is_symlink: false,
                is_directory: false,
                hash: sps2_hash::Hash::from_data(b"bin/x"),
            },
            sps2_hash::FileHashResult {
                relative_path: "share/d".to_string(),
                size: 2,
                mode: Some(0o644),
                is_symlink: false,
                is_directory: false,
                hash: sps2_hash::Hash::from_data(b"share/d"),
            },
        ];
        let td = TransactionData {
            package_refs: &[pref],
            file_references: &[],
            pending_file_hashes: &[(
                sps2_resolver::PackageId::new(
                    "B".to_string(),
                    sps2_types::Version::parse("1.0.0").unwrap(),
                ),
                file_hashes,
            )],
        };
        let staging_path = state.state_path().join(format!("staging-{staging_id}"));
        let _ = state
            .prepare_transaction(&staging_id, &parent_id, &staging_path, "install", &td)
            .await
            .expect("prepare");

        assert_eq!(store_ref_count(&state, &pkg_hash).await, 1);
        assert_eq!(
            file_obj_ref_count(&state, &sps2_hash::Hash::from_data(b"bin/x").to_hex()).await,
            1
        );
        assert_eq!(
            file_obj_ref_count(&state, &sps2_hash::Hash::from_data(b"share/d").to_hex()).await,
            1
        );
    }

    #[tokio::test]
    async fn t_2pc_uninstall_decrements_to_zero() {
        let (_td, state) = mk_state().await;
        let parent_id = state.get_active_state().await.expect("parent");
        let pkg_hash = sps2_hash::Hash::from_data(b"pkg-U").to_hex();
        seed_parent_with_pkg(&state, "U", "1.0.0", &pkg_hash, &[("bin/u", 3)]).await;

        let td = TransactionData {
            package_refs: &[],
            file_references: &[],
            pending_file_hashes: &[],
        };
        let staging_id = uuid::Uuid::new_v4();
        let staging_path = state.state_path().join(format!("staging-{staging_id}"));
        let _ = state
            .prepare_transaction(&staging_id, &parent_id, &staging_path, "uninstall", &td)
            .await
            .expect("prepare");

        assert_eq!(store_ref_count(&state, &pkg_hash).await, 0);
        assert_eq!(
            file_obj_ref_count(&state, &sps2_hash::Hash::from_data(b"bin/u").to_hex()).await,
            0
        );
    }

    #[tokio::test]
    async fn t_2pc_update_swaps_refs() {
        let (_td, state) = mk_state().await;
        let parent_id = state.get_active_state().await.expect("parent");
        let pkg_hash_v1 = sps2_hash::Hash::from_data(b"pkg-V1").to_hex();
        seed_parent_with_pkg(&state, "V", "1.0.0", &pkg_hash_v1, &[("bin/v1", 1)]).await;

        let pkg_hash_v2 = sps2_hash::Hash::from_data(b"pkg-V2").to_hex();
        let staging_id = uuid::Uuid::new_v4();
        let pid = sps2_resolver::PackageId::new(
            "V".to_string(),
            sps2_types::Version::parse("2.0.0").unwrap(),
        );
        let pref = PackageRef {
            state_id: staging_id,
            package_id: pid.clone(),
            hash: pkg_hash_v2.clone(),
            size: 1,
        };
        let fh = sps2_hash::FileHashResult {
            relative_path: "bin/v2".to_string(),
            size: 1,
            mode: Some(0o755),
            is_symlink: false,
            is_directory: false,
            hash: sps2_hash::Hash::from_data(b"bin/v2"),
        };
        let td = TransactionData {
            package_refs: &[pref],
            file_references: &[],
            pending_file_hashes: &[(pid, vec![fh])],
        };
        let staging_path = state.state_path().join(format!("staging-{staging_id}"));
        let _ = state
            .prepare_transaction(&staging_id, &parent_id, &staging_path, "update", &td)
            .await
            .expect("prepare");

        assert_eq!(store_ref_count(&state, &pkg_hash_v1).await, 0);
        assert_eq!(store_ref_count(&state, &pkg_hash_v2).await, 1);
        assert_eq!(
            file_obj_ref_count(&state, &sps2_hash::Hash::from_data(b"bin/v1").to_hex()).await,
            0
        );
        assert_eq!(
            file_obj_ref_count(&state, &sps2_hash::Hash::from_data(b"bin/v2").to_hex()).await,
            1
        );
    }
}

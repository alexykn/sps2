//! Atomic installer implementation using slot-based staging.

use crate::atomic::{package, transition::StateTransition};
// Removed Python venv handling - Python packages are now handled like regular packages
use crate::{InstallContext, InstallResult, PreparedPackage};
use sps2_errors::{Error, InstallError};
use sps2_events::events::{StateTransitionContext, TransitionSummary};
use sps2_events::{
    AppEvent, EventEmitter, EventSender, FailureContext, StateEvent, UninstallEvent,
};
use sps2_hash::Hash;
use sps2_platform::filesystem_helpers as fs_helpers;

use sps2_resolver::{PackageId, ResolvedNode};
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::time::Instant;
use uuid::Uuid;

/// Implement `EventEmitter` for `InstallContext`
impl EventEmitter for InstallContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

/// Implement `EventEmitter` for `UninstallContext`
impl EventEmitter for crate::UninstallContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

/// Implement `EventEmitter` for `UpdateContext`
impl EventEmitter for crate::UpdateContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

/// Atomic installer using APFS optimizations
pub struct AtomicInstaller {
    /// State manager for atomic transitions
    state_manager: StateManager,
    /// Content-addressable package store
    store: PackageStore,
}

impl AtomicInstaller {
    /// Execute two-phase commit flow for a transition
    async fn execute_two_phase_commit<T: EventEmitter>(
        &self,
        transition: &StateTransition,
        context: &T,
    ) -> Result<(), Error> {
        let source = transition.parent_id;
        let target = transition.staging_id;
        let parent_id = transition.parent_id.ok_or_else(|| {
            Error::from(InstallError::AtomicOperationFailed {
                message: "state transition missing parent state".to_string(),
            })
        })?;

        let transition_context = StateTransitionContext {
            operation: transition.operation.clone(),
            source,
            target,
        };
        let transition_start = Instant::now();

        context.emit(AppEvent::State(StateEvent::TransitionStarted {
            context: transition_context.clone(),
        }));

        let transition_data = sps2_state::TransactionData {
            package_refs: &transition.package_refs,
            file_references: &transition.file_references,
            pending_file_hashes: &transition.pending_file_hashes,
        };

        let journal = match self
            .state_manager
            .prepare_transaction(
                &transition.staging_id,
                &parent_id,
                transition.staging_slot,
                &transition.operation,
                &transition_data,
            )
            .await
        {
            Ok(journal) => journal,
            Err(e) => {
                let failure = FailureContext::from_error(&e);
                context.emit(AppEvent::State(StateEvent::TransitionFailed {
                    context: transition_context.clone(),
                    failure,
                }));
                return Err(e);
            }
        };

        if let Err(e) = self
            .state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await
        {
            let failure = FailureContext::from_error(&e);
            context.emit(AppEvent::State(StateEvent::TransitionFailed {
                context: transition_context.clone(),
                failure,
            }));
            return Err(e);
        }

        let summary = TransitionSummary {
            duration_ms: Some(
                u64::try_from(transition_start.elapsed().as_millis()).unwrap_or(u64::MAX),
            ),
        };

        context.emit(AppEvent::State(StateEvent::TransitionCompleted {
            context: transition_context,
            summary: Some(summary),
        }));

        Ok(())
    }

    /// Setup state transition and staging directory
    async fn setup_state_transition<T: EventEmitter>(
        &self,
        operation: &str,
        context: &T,
    ) -> Result<StateTransition, Error> {
        // Create new state transition
        let mut transition =
            StateTransition::new(&self.state_manager, operation.to_string()).await?;

        // Set event sender on transition
        transition.event_sender = context.event_sender().cloned();

        context.emit_debug(format!(
            "Prepared staging slot {} at {}",
            transition.staging_slot,
            transition.slot_path.display()
        ));

        Ok(transition)
    }

    /// Create new atomic installer
    ///
    /// # Errors
    ///
    /// Returns an error if initialization fails
    #[must_use]
    pub fn new(state_manager: StateManager, store: PackageStore) -> Self {
        Self {
            state_manager,
            store,
        }
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
        prepared_packages: Option<&HashMap<PackageId, PreparedPackage>>,
    ) -> Result<InstallResult, Error> {
        // Setup state transition and staging directory
        let mut transition = self.setup_state_transition("install", context).await?;

        // Collect the current state's packages so we can carry forward untouched entries and
        // detect in-place upgrades cleanly.
        let parent_packages = if let Some(parent_id) = transition.parent_id {
            self.state_manager
                .get_installed_packages_in_state(&parent_id)
                .await?
        } else {
            Vec::new()
        };
        let parent_lookup: HashMap<String, sps2_state::models::Package> = parent_packages
            .iter()
            .cloned()
            .map(|pkg| (pkg.name.clone(), pkg))
            .collect();

        package::sync_slot_with_parent(
            &self.state_manager,
            &self.store,
            &mut transition,
            &parent_packages,
        )
        .await?;

        // The staging slot now mirrors the parent state, so carry-forward only needs to
        // register package references for unchanged packages.
        let exclude_names: HashSet<String> = resolved_packages
            .keys()
            .map(|pkg| pkg.name.clone())
            .collect();
        package::carry_forward_packages(&mut transition, &parent_packages, &exclude_names);

        // Apply package changes to staging
        let mut result = InstallResult::new(transition.staging_id);

        for (package_id, node) in resolved_packages {
            let prepared_package = prepared_packages.and_then(|packages| packages.get(package_id));
            package::install_package_to_staging(
                &self.state_manager,
                &mut transition,
                package_id,
                node,
                prepared_package,
                parent_lookup.get(&package_id.name),
                &mut result,
            )
            .await?;
        }

        // Execute two-phase commit
        self.execute_two_phase_commit(&transition, context).await?;

        Ok(result)
    }

    // Removed install_python_package - Python packages are now handled like regular packages

    /// Perform atomic uninstallation
    ///
    /// # Errors
    ///
    /// Returns an error if state transition fails, package removal fails,
    /// or filesystem operations fail.
    pub async fn uninstall(
        &mut self,
        packages_to_remove: &[PackageId],
        context: &crate::UninstallContext,
    ) -> Result<InstallResult, Error> {
        // Setup state transition and staging directory
        let mut transition = self.setup_state_transition("uninstall", context).await?;

        // Ensure the staging slot mirrors the current parent state before applying removals.
        let mut result = InstallResult::new(transition.staging_id);

        for pkg in packages_to_remove {
            context.emit(AppEvent::Uninstall(UninstallEvent::Started {
                package: pkg.name.clone(),
                version: pkg.version.clone(),
            }));
        }

        // Remove packages from staging and track them in result
        let parent_packages = if let Some(parent_id) = transition.parent_id {
            self.state_manager
                .get_installed_packages_in_state(&parent_id)
                .await?
        } else {
            Vec::new()
        };

        package::sync_slot_with_parent(
            &self.state_manager,
            &self.store,
            &mut transition,
            &parent_packages,
        )
        .await?;

        for pkg in &parent_packages {
            // Check if this package should be removed
            let should_remove = packages_to_remove
                .iter()
                .any(|remove_pkg| remove_pkg.name == pkg.name);

            if should_remove {
                result.add_removed(PackageId::new(pkg.name.clone(), pkg.version()));
                package::remove_package_from_staging(&self.state_manager, &mut transition, pkg)
                    .await?;
                context.emit_debug(format!("Removed package {} from staging", pkg.name));
            }
        }

        // Carry forward packages that are not being removed
        let exclude_names: HashSet<String> = packages_to_remove
            .iter()
            .map(|pkg| pkg.name.clone())
            .collect();
        package::carry_forward_packages(&mut transition, &parent_packages, &exclude_names);

        // Execute two-phase commit
        self.execute_two_phase_commit(&transition, context).await?;

        for pkg in &result.removed_packages {
            context.emit(AppEvent::Uninstall(UninstallEvent::Completed {
                package: pkg.name.clone(),
                version: pkg.version.clone(),
                files_removed: 0,
            }));
        }

        Ok(result)
    }

    // Removed remove_package_venv - Python packages are now handled like regular packages

    /// Rollback by moving active to an existing target state without creating a new state row
    ///
    /// # Errors
    ///
    /// Returns an error if staging or filesystem operations fail.
    pub async fn rollback_move_to_state(&mut self, target_state_id: Uuid) -> Result<(), Error> {
        let current_state_id = self.state_manager.get_current_state_id().await?;

        let mut transition =
            StateTransition::new(&self.state_manager, "rollback".to_string()).await?;
        let target_packages = self
            .state_manager
            .get_installed_packages_in_state(&target_state_id)
            .await?;

        fs_helpers::ensure_empty_dir(&transition.slot_path).await?;

        for pkg in &target_packages {
            let hash = Hash::from_hex(&pkg.hash).map_err(|e| {
                sps2_errors::Error::internal(format!("invalid hash {}: {e}", pkg.hash))
            })?;
            let store_path = self.store.package_path(&hash);
            let pkg_id = PackageId::new(pkg.name.clone(), pkg.version());
            crate::atomic::fs::link_package_to_staging(
                &mut transition,
                &store_path,
                &pkg_id,
                false,
            )
            .await?;
        }

        self.state_manager
            .set_slot_state(transition.staging_slot, Some(target_state_id))
            .await?;

        let journal = sps2_types::state::TransactionJournal {
            new_state_id: target_state_id,
            parent_state_id: current_state_id,
            staging_path: transition.slot_path.clone(),
            staging_slot: transition.staging_slot,
            phase: sps2_types::state::TransactionPhase::Prepared,
            operation: "rollback".to_string(),
        };
        self.state_manager.write_journal(&journal).await?;
        self.state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await?;

        // After switching active state, synchronize DB refcounts to match the target state exactly
        let _ = self
            .state_manager
            .sync_refcounts_to_state(&target_state_id)
            .await?;

        // Ensure the target state is visible in base history
        self.state_manager.unprune_state(&target_state_id).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_store::create_package;
    use sps2_types::{Arch, Manifest, Version};
    use std::collections::HashMap;
    use tempfile::TempDir;
    use tokio::fs as afs;

    async fn mk_env() -> (TempDir, StateManager, sps2_store::PackageStore) {
        let td = TempDir::new().expect("td");
        let state = StateManager::new(td.path()).await.expect("state");
        let store_base = td.path().join("store");
        afs::create_dir_all(&store_base).await.unwrap();
        let store = sps2_store::PackageStore::new(store_base);
        (td, state, store)
    }

    async fn make_sp_and_add_to_store(
        store: &sps2_store::PackageStore,
        name: &str,
        version: &str,
        files: &[(&str, &str)],
    ) -> (
        sps2_hash::Hash,
        std::path::PathBuf,
        u64,
        Vec<sps2_hash::Hash>,
    ) {
        let td = TempDir::new().unwrap();
        let src = td.path().join("src");
        afs::create_dir_all(&src).await.unwrap();
        // manifest
        let v = Version::parse(version).unwrap();
        let m = Manifest::new(name.to_string(), &v, 1, &Arch::Arm64);
        let manifest_path = src.join("manifest.toml");
        sps2_store::manifest_io::write_manifest(&manifest_path, &m)
            .await
            .unwrap();
        // files under opt/pm/live
        for (rel, content) in files {
            let p = src.join("opt/pm/live").join(rel);
            if let Some(parent) = p.parent() {
                afs::create_dir_all(parent).await.unwrap();
            }
            afs::write(&p, content.as_bytes()).await.unwrap();
        }
        // create .sp
        let sp = td.path().join("pkg.sp");
        create_package(&src, &sp).await.unwrap();
        // add to store
        let stored = store.add_package(&sp).await.unwrap();
        let hash = stored.hash().unwrap();
        let path = store.package_path(&hash);
        let file_hashes: Vec<sps2_hash::Hash> = stored
            .file_hashes()
            .unwrap_or(&[])
            .iter()
            .map(|r| r.hash.clone())
            .collect();
        let size = afs::metadata(&sp).await.unwrap().len();
        (hash, path, size, file_hashes)
    }

    fn collect_relative_files(base: &std::path::Path) -> Vec<std::path::PathBuf> {
        fn walk(
            base: &std::path::Path,
            current: &std::path::Path,
            acc: &mut Vec<std::path::PathBuf>,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(current)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    walk(base, &path, acc)?;
                } else {
                    acc.push(path.strip_prefix(base).unwrap().to_path_buf());
                }
            }
            Ok(())
        }

        let mut acc = Vec::new();
        walk(base, base, &mut acc).unwrap();
        acc
    }

    async fn refcount_store(state: &StateManager, hex: &str) -> i64 {
        let mut tx = state.begin_transaction().await.unwrap();
        let all = sps2_state::queries::get_all_store_refs(&mut tx)
            .await
            .unwrap();
        all.into_iter()
            .find(|r| r.hash == hex)
            .map_or(0, |r| r.ref_count)
    }
    async fn refcount_file(state: &StateManager, hex: &str) -> i64 {
        let mut tx = state.begin_transaction().await.unwrap();
        let h = sps2_hash::Hash::from_hex(hex).unwrap();
        let row = sps2_state::file_queries_runtime::get_file_object(&mut tx, &h)
            .await
            .unwrap();
        row.map_or(0, |o| o.ref_count)
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Integration test with comprehensive setup
    async fn cloned_staging_carries_forward_package_files() {
        let (_td, state, store) = mk_env().await;
        let (hash_a, path_a, size_a, _file_hashes_a) = make_sp_and_add_to_store(
            &store,
            "A",
            "1.0.0",
            &[("bin/a", "alpha"), ("share/doc.txt", "alpha docs")],
        )
        .await;
        let (hash_b, path_b, size_b, _file_hashes_b) = make_sp_and_add_to_store(
            &store,
            "B",
            "1.0.0",
            &[("bin/b", "beta"), ("share/readme.txt", "beta docs")],
        )
        .await;

        let mut installer = AtomicInstaller::new(state.clone(), store.clone());

        // Install package A
        let mut resolved_a: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_a = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved_a.insert(
            pid_a.clone(),
            ResolvedNode::local(
                pid_a.name.clone(),
                pid_a.version.clone(),
                path_a.clone(),
                vec![],
            ),
        );
        let mut prepared_a = HashMap::new();
        prepared_a.insert(
            pid_a.clone(),
            crate::PreparedPackage {
                hash: hash_a.clone(),
                size: size_a,
                store_path: path_a.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _ = installer
            .install(&ctx, &resolved_a, Some(&prepared_a))
            .await
            .unwrap();

        // Install package B (forces cloned staging when clone succeeds)
        let mut resolved_b: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_b = PackageId::new(
            "B".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved_b.insert(
            pid_b.clone(),
            ResolvedNode::local(
                pid_b.name.clone(),
                pid_b.version.clone(),
                path_b.clone(),
                vec![],
            ),
        );
        let mut prepared_b = HashMap::new();
        prepared_b.insert(
            pid_b.clone(),
            crate::PreparedPackage {
                hash: hash_b.clone(),
                size: size_b,
                store_path: path_b.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx_b = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _ = installer
            .install(&ctx_b, &resolved_b, Some(&prepared_b))
            .await
            .unwrap();

        let active_state = state.get_current_state_id().await.unwrap();
        let mut tx = state.begin_transaction().await.unwrap();
        let pkg_a_files = sps2_state::file_queries_runtime::get_package_file_entries_by_name(
            &mut tx,
            &active_state,
            "A",
            "1.0.0",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert!(
            !pkg_a_files.is_empty(),
            "package_files entries for package A should be preserved after cloned staging"
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Integration test with comprehensive setup
    async fn install_then_update_replaces_old_version() {
        let (_td, state, store) = mk_env().await;
        let (hash_v1, path_v1, size_v1, _file_hashes_v1) = make_sp_and_add_to_store(
            &store,
            "A",
            "1.0.0",
            &[("share/v1.txt", "v1"), ("bin/a", "binary")],
        )
        .await;
        let (hash_v2, path_v2, size_v2, _file_hashes_v2) = make_sp_and_add_to_store(
            &store,
            "A",
            "1.1.0",
            &[("share/v2.txt", "v2"), ("bin/a", "binary2")],
        )
        .await;

        let mut ai = AtomicInstaller::new(state.clone(), store.clone());

        // Initial install of v1
        let mut resolved: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_v1 = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved.insert(
            pid_v1.clone(),
            ResolvedNode::local(
                "A".to_string(),
                pid_v1.version.clone(),
                path_v1.clone(),
                vec![],
            ),
        );
        let mut prepared = HashMap::new();
        prepared.insert(
            pid_v1.clone(),
            crate::PreparedPackage {
                hash: hash_v1.clone(),
                size: size_v1,
                store_path: path_v1.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _ = ai.install(&ctx, &resolved, Some(&prepared)).await.unwrap();

        let live_path = state.live_path().to_path_buf();
        let files_after_install = collect_relative_files(&live_path);
        assert!(files_after_install
            .iter()
            .any(|p| p.ends_with("share/v1.txt")));
        let binary_rel_initial = files_after_install
            .iter()
            .find(|p| p.ends_with("bin/a"))
            .expect("binary present after initial install");
        assert_eq!(
            std::fs::read_to_string(live_path.join(binary_rel_initial)).unwrap(),
            "binary"
        );

        // Update to v2
        let mut resolved_update: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_v2 = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.1.0").unwrap(),
        );
        resolved_update.insert(
            pid_v2.clone(),
            ResolvedNode::local(
                "A".to_string(),
                pid_v2.version.clone(),
                path_v2.clone(),
                vec![],
            ),
        );
        let mut prepared_update = HashMap::new();
        prepared_update.insert(
            pid_v2.clone(),
            crate::PreparedPackage {
                hash: hash_v2.clone(),
                size: size_v2,
                store_path: path_v2.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let update_ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: true,
            force_download: false,
            event_sender: None,
        };
        let update_result = ai
            .install(&update_ctx, &resolved_update, Some(&prepared_update))
            .await
            .unwrap();

        assert!(update_result.installed_packages.is_empty());
        assert_eq!(update_result.updated_packages, vec![pid_v2.clone()]);
        assert!(update_result.removed_packages.is_empty());

        let installed = state.get_installed_packages().await.unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].version, "1.1.0");

        // Live directory should reflect the new version
        let files_after_update = collect_relative_files(&live_path);
        assert!(!files_after_update
            .iter()
            .any(|p| p.ends_with("share/v1.txt")));
        assert!(files_after_update
            .iter()
            .any(|p| p.ends_with("share/v2.txt")));
        let binary_rel_updated = files_after_update
            .iter()
            .find(|p| p.ends_with("bin/a"))
            .expect("binary present after update");
        assert_eq!(
            std::fs::read_to_string(live_path.join(binary_rel_updated)).unwrap(),
            "binary2"
        );

        assert_eq!(refcount_store(&state, &hash_v1.to_hex()).await, 0);
        assert!(refcount_store(&state, &hash_v2.to_hex()).await > 0);
    }

    #[tokio::test]
    async fn install_then_uninstall_updates_refcounts() {
        let (_td, state, store) = mk_env().await;
        let (hash, store_path, size, file_hashes) =
            make_sp_and_add_to_store(&store, "A", "1.0.0", &[("bin/x", "same"), ("share/a", "A")])
                .await;

        let mut ai = AtomicInstaller::new(state.clone(), store.clone());
        let mut resolved: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved.insert(
            pid.clone(),
            ResolvedNode::local(
                "A".to_string(),
                pid.version.clone(),
                store_path.clone(),
                vec![],
            ),
        );
        let mut prepared = HashMap::new();
        prepared.insert(
            pid.clone(),
            crate::PreparedPackage {
                hash: hash.clone(),
                size,
                store_path: store_path.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _res = ai.install(&ctx, &resolved, Some(&prepared)).await.unwrap();

        // After install
        assert!(refcount_store(&state, &hash.to_hex()).await > 0);
        for fh in &file_hashes {
            assert!(refcount_file(&state, &fh.to_hex()).await > 0);
        }

        // Uninstall package A
        let uctx = crate::UninstallContext {
            packages: vec!["A".to_string()],
            autoremove: false,
            force: true,
            event_sender: None,
        };
        let _u = ai
            .uninstall(
                &[PackageId::new(
                    "A".to_string(),
                    sps2_types::Version::parse("1.0.0").unwrap(),
                )],
                &uctx,
            )
            .await
            .unwrap();

        assert_eq!(refcount_store(&state, &hash.to_hex()).await, 0);
        for fh in &file_hashes {
            assert_eq!(refcount_file(&state, &fh.to_hex()).await, 0);
        }
    }

    #[tokio::test]
    async fn shared_file_uninstall_decrements_but_not_zero() {
        let (_td, state, store) = mk_env().await;
        // A and B share bin/x
        let (hash_a, path_a, size_a, file_hashes_a) = make_sp_and_add_to_store(
            &store,
            "A",
            "1.0.0",
            &[("bin/x", "same"), ("share/a", "AA")],
        )
        .await;
        let (hash_b, path_b, size_b, file_hashes_b) = make_sp_and_add_to_store(
            &store,
            "B",
            "1.0.0",
            &[("bin/x", "same"), ("share/b", "BB")],
        )
        .await;
        let h_same = file_hashes_a
            .iter()
            .find(|h| file_hashes_b.iter().any(|hb| hb == *h))
            .unwrap()
            .clone();

        let mut ai = AtomicInstaller::new(state.clone(), store.clone());

        // Install A then B
        let mut resolved: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_a = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        let pid_b = PackageId::new(
            "B".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved.insert(
            pid_a.clone(),
            ResolvedNode::local(
                "A".to_string(),
                pid_a.version.clone(),
                path_a.clone(),
                vec![],
            ),
        );
        resolved.insert(
            pid_b.clone(),
            ResolvedNode::local(
                "B".to_string(),
                pid_b.version.clone(),
                path_b.clone(),
                vec![],
            ),
        );

        let mut prepared = HashMap::new();
        prepared.insert(
            pid_a.clone(),
            crate::PreparedPackage {
                hash: hash_a.clone(),
                size: size_a,
                store_path: path_a.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        prepared.insert(
            pid_b.clone(),
            crate::PreparedPackage {
                hash: hash_b.clone(),
                size: size_b,
                store_path: path_b.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _res = ai.install(&ctx, &resolved, Some(&prepared)).await.unwrap();

        // Uninstall A
        let uctx = crate::UninstallContext {
            packages: vec!["A".to_string()],
            autoremove: false,
            force: true,
            event_sender: None,
        };
        let _u = ai
            .uninstall(std::slice::from_ref(&pid_a), &uctx)
            .await
            .unwrap();

        // Shared file remains referenced by B
        assert!(refcount_file(&state, &h_same.to_hex()).await > 0);
    }
}

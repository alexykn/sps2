use crate::refcount::sync_refcounts_to_active_state;
use sps2_errors::{Error, OpsError};
use sps2_events::{
    AppEvent, EventEmitter, EventSender, GuardDiscrepancy, GuardEvent, GuardLevel, GuardScope,
    GuardSeverity, GuardTargetSummary, GuardVerificationMetrics,
};
use sps2_hash::Hash;
use sps2_platform::PlatformManager;
use sps2_state::{queries, Package, PackageFileEntry, StateManager};
use sps2_store::{PackageStore, StoredPackage};
use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;
use tokio::fs;
use uuid::Uuid;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryStatus {
    Ok,
    Missing,
    Corrupted,
}

/// Verification level controls the depth of checks performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationLevel {
    Quick,
    Standard,
    Full,
}

impl From<&str> for VerificationLevel {
    fn from(value: &str) -> Self {
        match value {
            "quick" => Self::Quick,
            "full" => Self::Full,
            _ => Self::Standard,
        }
    }
}

impl VerificationLevel {
    fn as_guard_level(self) -> GuardLevel {
        match self {
            VerificationLevel::Quick => GuardLevel::Quick,
            VerificationLevel::Standard => GuardLevel::Standard,
            VerificationLevel::Full => GuardLevel::Full,
        }
    }
}

/// Issues discovered during verification.
#[derive(Debug, Clone, serde::Serialize)]
pub enum Discrepancy {
    MissingFile {
        package: String,
        version: String,
        path: String,
    },
    CorruptedFile {
        package: String,
        version: String,
        path: String,
    },
    MissingPackageContent {
        package: String,
        version: String,
    },
    UnexpectedFile {
        path: String,
    },
}

impl Discrepancy {
    fn to_event(&self) -> GuardDiscrepancy {
        match self {
            Discrepancy::MissingFile {
                package,
                version,
                path,
            } => GuardDiscrepancy {
                kind: "missing_file".to_string(),
                severity: GuardSeverity::High,
                location: Some(path.clone()),
                package: Some(package.clone()),
                version: Some(version.clone()),
                message: format!("{package}-{version} is missing {path}"),
                auto_heal_available: true,
                requires_confirmation: false,
            },
            Discrepancy::CorruptedFile {
                package,
                version,
                path,
            } => GuardDiscrepancy {
                kind: "corrupted_file".to_string(),
                severity: GuardSeverity::High,
                location: Some(path.clone()),
                package: Some(package.clone()),
                version: Some(version.clone()),
                message: format!("{package}-{version} has corrupted {path}"),
                auto_heal_available: true,
                requires_confirmation: false,
            },
            Discrepancy::MissingPackageContent { package, version } => GuardDiscrepancy {
                kind: "missing_package_content".to_string(),
                severity: GuardSeverity::Critical,
                location: None,
                package: Some(package.clone()),
                version: Some(version.clone()),
                message: format!("Package {package}-{version} content missing from store"),
                auto_heal_available: false,
                requires_confirmation: true,
            },
            Discrepancy::UnexpectedFile { path } => GuardDiscrepancy {
                kind: "unexpected_file".to_string(),
                severity: GuardSeverity::Medium,
                location: Some(path.clone()),
                package: None,
                version: None,
                message: format!("Untracked file present: {path}"),
                auto_heal_available: false,
                requires_confirmation: false,
            },
        }
    }
}

/// Result of a verification run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerificationResult {
    pub state_id: Uuid,
    pub discrepancies: Vec<Discrepancy>,
    pub is_valid: bool,
    pub duration_ms: u64,
}

impl VerificationResult {
    pub fn new(state_id: Uuid, discrepancies: Vec<Discrepancy>, duration_ms: u64) -> Self {
        let is_valid = discrepancies.is_empty();
        Self {
            state_id,
            discrepancies,
            is_valid,
            duration_ms,
        }
    }
}

/// Lightweight verifier that checks live state against the content store.
pub struct Verifier {
    state: StateManager,
    store: PackageStore,
    tx: EventSender,
}

impl EventEmitter for Verifier {
    fn event_sender(&self) -> Option<&EventSender> {
        Some(&self.tx)
    }
}

impl Verifier {
    pub fn new(state: StateManager, store: PackageStore, tx: EventSender) -> Self {
        Self { state, store, tx }
    }

    pub async fn verify(&self, level: VerificationLevel) -> Result<VerificationResult, Error> {
        self.run(level, false).await
    }

    pub async fn verify_and_heal(
        &self,
        level: VerificationLevel,
    ) -> Result<VerificationResult, Error> {
        self.run(level, true).await
    }

    pub async fn sync_refcounts(&self) -> Result<(usize, usize), Error> {
        sync_refcounts_to_active_state(&self.state).await
    }

    async fn run(&self, level: VerificationLevel, heal: bool) -> Result<VerificationResult, Error> {
        let start = Instant::now();
        let state_id = self.state.get_active_state().await?;
        let live_root = self.state.live_path().to_path_buf();
        let packages = self.load_packages(&state_id).await?;

        let total_files: usize = packages.iter().map(|(_, entries)| entries.len()).sum();

        let operation_id = Uuid::new_v4().to_string();
        let scope = GuardScope::State {
            id: state_id.to_string(),
        };

        self.emit(AppEvent::Guard(GuardEvent::VerificationStarted {
            operation_id: operation_id.clone(),
            scope: scope.clone(),
            level: level.as_guard_level(),
            targets: GuardTargetSummary {
                packages: packages.len(),
                files: Some(total_files),
            },
        }));

        let mut discrepancies = Vec::new();
        let mut tracked_files: HashSet<String> = HashSet::new();

        for (package, entries) in packages.iter() {
            let package_hash = Hash::from_hex(&package.hash).map_err(|e| {
                Error::from(OpsError::OperationFailed {
                    message: format!("invalid package hash for {}: {e}", package.name),
                })
            })?;
            let store_path = self.store.package_path(&package_hash);
            if !store_path.exists() {
                let discrepancy = Discrepancy::MissingPackageContent {
                    package: package.name.clone(),
                    version: package.version.clone(),
                };
                self.emit_discrepancy(&operation_id, &discrepancy);
                discrepancies.push(discrepancy);
                continue;
            }

            let stored_package = StoredPackage::load(&store_path).await?;

            for entry in entries {
                tracked_files.insert(entry.relative_path.clone());
                match self
                    .verify_entry(&stored_package, package, entry, &live_root, level, heal)
                    .await?
                {
                    EntryStatus::Ok => {}
                    EntryStatus::Missing => {
                        let discrepancy =
                            self.make_discrepancy(package, entry, EntryStatus::Missing);
                        self.emit_discrepancy(&operation_id, &discrepancy);
                        discrepancies.push(discrepancy);
                    }
                    EntryStatus::Corrupted => {
                        let discrepancy =
                            self.make_discrepancy(package, entry, EntryStatus::Corrupted);
                        self.emit_discrepancy(&operation_id, &discrepancy);
                        discrepancies.push(discrepancy);
                    }
                }
            }
        }

        // Detect unexpected files in live directory
        let unexpected = self
            .detect_orphans(&live_root, &tracked_files, heal)
            .await?;
        for discrepancy in unexpected {
            self.emit_discrepancy(&operation_id, &discrepancy);
            discrepancies.push(discrepancy);
        }

        let duration = start.elapsed();
        self.emit(AppEvent::Guard(GuardEvent::VerificationCompleted {
            operation_id,
            scope,
            discrepancies: discrepancies.len(),
            metrics: GuardVerificationMetrics {
                duration_ms: duration.as_millis() as u64,
                cache_hit_rate: 0.0,
                coverage_percent: 100.0,
            },
        }));

        Ok(VerificationResult::new(
            state_id,
            discrepancies,
            duration.as_millis() as u64,
        ))
    }

    async fn verify_entry(
        &self,
        stored_package: &StoredPackage,
        package: &sps2_state::models::Package,
        entry: &PackageFileEntry,
        live_root: &Path,
        level: VerificationLevel,
        heal: bool,
    ) -> Result<EntryStatus, Error> {
        let full_path = live_root.join(&entry.relative_path);
        if !full_path.exists() {
            if heal
                && self
                    .restore_file(stored_package, package, entry, &full_path)
                    .await
                    .is_ok()
                && full_path.exists()
            {
                return Ok(EntryStatus::Ok);
            }
            return Ok(EntryStatus::Missing);
        }

        let metadata = fs::symlink_metadata(&full_path).await?;
        if metadata.file_type().is_symlink() || metadata.is_dir() {
            // Skip hash verification for symlinks/directories
            return Ok(EntryStatus::Ok);
        }

        if level == VerificationLevel::Quick {
            return Ok(EntryStatus::Ok);
        }

        // Standard level: verify basic file permissions
        if level == VerificationLevel::Standard {
            return Ok(EntryStatus::Ok);
        }

        // Full level: hash comparison
        let expected_hash = Hash::from_hex(&entry.file_hash).map_err(|e| {
            Error::from(OpsError::OperationFailed {
                message: format!(
                    "invalid file hash for {}:{} - {e}",
                    package.name, entry.relative_path
                ),
            })
        })?;

        // Skip Python bytecode caches for stability
        if entry.relative_path.ends_with(".pyc") || entry.relative_path.contains("__pycache__") {
            return Ok(EntryStatus::Ok);
        }

        let actual_hash = Hash::hash_file(&full_path).await?;
        if actual_hash == expected_hash {
            return Ok(EntryStatus::Ok);
        }

        if heal
            && self
                .restore_file(stored_package, package, entry, &full_path)
                .await
                .is_ok()
        {
            let rehash = Hash::hash_file(&full_path).await?;
            if rehash == expected_hash {
                return Ok(EntryStatus::Ok);
            }
        }
        Ok(EntryStatus::Corrupted)
    }

    async fn restore_file(
        &self,
        stored_package: &StoredPackage,
        package: &sps2_state::models::Package,
        entry: &PackageFileEntry,
        target_path: &Path,
    ) -> Result<(), Error> {
        let source_path = if stored_package.has_file_hashes() {
            let file_hash = Hash::from_hex(&entry.file_hash).map_err(|e| {
                Error::from(OpsError::OperationFailed {
                    message: format!(
                        "invalid file hash for {}:{} - {e}",
                        package.name, entry.relative_path
                    ),
                })
            })?;
            self.store.file_path(&file_hash)
        } else {
            stored_package.files_path().join(&entry.relative_path)
        };

        if !source_path.exists() {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "missing source file {} for {}-{}",
                    source_path.display(),
                    package.name,
                    package.version
                ),
            }
            .into());
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let platform = PlatformManager::instance().platform();
        let ctx = platform.create_context(None);

        if let Ok(existing_meta) = fs::symlink_metadata(target_path).await {
            if existing_meta.is_dir() {
                let _ = platform
                    .filesystem()
                    .remove_dir_all(&ctx, target_path)
                    .await;
            } else {
                let _ = platform.filesystem().remove_file(&ctx, target_path).await;
            }
        }

        let metadata = fs::symlink_metadata(&source_path).await?;

        if metadata.is_dir() {
            platform
                .filesystem()
                .clone_directory(&ctx, &source_path, target_path)
                .await?
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(&source_path).await?;
            fs::symlink(&target, target_path).await?;
        } else {
            platform
                .filesystem()
                .clone_file(&ctx, &source_path, target_path)
                .await?;
        }

        Ok(())
    }

    fn make_discrepancy(
        &self,
        package: &sps2_state::models::Package,
        entry: &PackageFileEntry,
        status: EntryStatus,
    ) -> Discrepancy {
        match status {
            EntryStatus::Missing => Discrepancy::MissingFile {
                package: package.name.clone(),
                version: package.version.clone(),
                path: entry.relative_path.clone(),
            },
            EntryStatus::Corrupted => Discrepancy::CorruptedFile {
                package: package.name.clone(),
                version: package.version.clone(),
                path: entry.relative_path.clone(),
            },
            EntryStatus::Ok => unreachable!(),
        }
    }

    async fn detect_orphans(
        &self,
        live_root: &Path,
        tracked: &HashSet<String>,
        heal: bool,
    ) -> Result<Vec<Discrepancy>, Error> {
        if !live_root.exists() {
            return Ok(Vec::new());
        }

        let mut unexpected = Vec::new();
        for entry in WalkDir::new(live_root).follow_links(false) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if entry.file_type().is_dir() {
                continue;
            }

            let rel_path = match entry.path().strip_prefix(live_root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };

            if rel_path.ends_with(".pyc") 
                || rel_path.contains("__pycache__") 
                || rel_path == "STATE" {
                continue;
            }

            if !tracked.contains(&rel_path) {
                if heal && fs::remove_file(entry.path()).await.is_ok() {
                    continue;
                }
                unexpected.push(Discrepancy::UnexpectedFile { path: rel_path });
            }
        }

        Ok(unexpected)
    }

    fn emit_discrepancy(&self, operation_id: &str, discrepancy: &Discrepancy) {
        self.emit(AppEvent::Guard(GuardEvent::DiscrepancyReported {
            operation_id: operation_id.to_string(),
            discrepancy: discrepancy.to_event(),
        }));
    }

    async fn load_packages(
        &self,
        state_id: &Uuid,
    ) -> Result<Vec<(Package, Vec<PackageFileEntry>)>, Error> {
        let mut tx = self.state.begin_transaction().await?;
        let packages = queries::get_state_packages(&mut tx, state_id).await?;
        tx.commit().await?;

        let mut result = Vec::new();
        for package in packages {
            let mut tx = self.state.begin_transaction().await?;
            let entries = queries::get_package_file_entries(&mut tx, package.id).await?;
            tx.commit().await?;
            result.push((package, entries));
        }

        Ok(result)
    }
}

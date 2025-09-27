//! Staging and installation pipeline stage

use crate::pipeline::decompress::DecompressResult;
// use crate::staging::StagingManager; // removed in slot-based implementation
use sps2_errors::Error;
use sps2_events::{AppEvent, EventEmitter, EventSender, GeneralEvent, InstallEvent};
use sps2_resolver::PackageId;
use sps2_store::PackageStore;
use tokio::task::JoinHandle;

struct StagingContext {
    event_sender: Option<EventSender>,
}

impl EventEmitter for StagingContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

/// Staging pipeline stage coordinator
pub struct StagingPipeline {
    store: PackageStore,
}

impl StagingPipeline {
    /// Create a new staging pipeline
    pub fn new(store: PackageStore) -> Self {
        Self {
            store,
        }
    }

    /// Execute parallel staging and installation
    pub async fn execute_parallel_staging_install(
        &self,
        decompress_results: &[DecompressResult],
        progress_id: &str,
        tx: &EventSender,
    ) -> Result<Vec<Result<PackageId, (PackageId, Error)>>, Error> {
        let mut handles = Vec::new();

        for decompress_result in decompress_results {
            // Need to move ownership of the decompress result
            let result_moved = DecompressResult {
                package_id: decompress_result.package_id.clone(),
                decompressed_path: decompress_result.decompressed_path.clone(),
                validation_result: decompress_result.validation_result.clone(),
                hash: decompress_result.hash.clone(),
                temp_file: None, // Can't clone NamedTempFile, so we don't pass it
            };

            let handle =
                self.spawn_staging_install_task(result_moved, progress_id.to_string(), tx.clone());
            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            let result = handle
                .await
                .map_err(|e| sps2_errors::InstallError::TaskError {
                    message: format!("staging/install task failed: {e}"),
                })?;
            results.push(result);
        }

        Ok(results)
    }

    /// Spawn staging and installation task
    fn spawn_staging_install_task(
        &self,
        decompress_result: DecompressResult,
        progress_id: String,
        tx: EventSender,
    ) -> JoinHandle<Result<PackageId, (PackageId, Error)>> {
        let store = self.store.clone();

        tokio::spawn(async move {
            match Self::stage_and_install_package(
                &decompress_result,
                &store,
                &tx,
                &progress_id,
            )
            .await
            {
                Ok(package_id) => Ok(package_id),
                Err(e) => Err((decompress_result.package_id, e)),
            }
        })
    }

    /// Stage and install a single package
    async fn stage_and_install_package(
        decompress_result: &DecompressResult,
        store: &PackageStore,
        tx: &EventSender,
        progress_id: &str,
    ) -> Result<PackageId, Error> {
        tx.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Starting staging/install for package: {}",
                decompress_result.package_id.name
            ),
            context: std::collections::HashMap::new(),
        }));

        // Extract to staging directory with progress tracking
        tx.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Extracting to staging directory from: {}",
                decompress_result.decompressed_path.display()
            ),
            context: std::collections::HashMap::new(),
        }));

        let staging_context = StagingContext {
            event_sender: Some(tx.clone()),
        };

        // In the slot-based implementation, the package has already been decompressed
        // and validated. We can add it to the store directly from decompressed_path.
        let stored_package = store
            .add_package_from_staging(&decompress_result.decompressed_path, &decompress_result.package_id)
            .await?;

        tx.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: "DEBUG: Decompressed package ready for store ingestion".to_string(),
            context: std::collections::HashMap::new(),
        }));

        tx.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: "DEBUG: Package added to store successfully".to_string(),
            context: std::collections::HashMap::new(),
        }));
        tx.emit(AppEvent::Install(InstallEvent::Completed {
            package: decompress_result.package_id.name.clone(),
            version: decompress_result.package_id.version.clone(),
            files_installed: decompress_result.validation_result.file_count,
        }));

        // Get the hash from the stored package
        let hash = stored_package
            .hash()
            .ok_or_else(|| sps2_errors::StorageError::IoError {
                message: "failed to get hash from stored package".to_string(),
            })?;

        tx.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Package installation completed: {} (hash: {})",
                decompress_result.package_id.name,
                hash.to_hex()
            ),
            context: std::collections::HashMap::new(),
        }));

        Ok(decompress_result.package_id.clone())
    }
}

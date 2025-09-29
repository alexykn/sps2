//! Worker functions for package processing

use crate::PreparedPackage;
use dashmap::DashMap;
use sps2_errors::{Error, InstallError};
use sps2_events::events::AcquisitionSource;
use sps2_events::{
    AcquisitionEvent, AppEvent, EventEmitter, FailureContext, GeneralEvent, InstallEvent,
};
use sps2_net::{PackageDownloadConfig, PackageDownloader};
use sps2_resolver::{NodeAction, PackageId, ResolvedNode};
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::sync::Arc;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Duration;

use super::context::ExecutionContext;

pub(crate) struct ProcessPackageArgs {
    pub package_id: PackageId,
    pub node: ResolvedNode,
    pub context: ExecutionContext,
    pub store: PackageStore,
    pub state_manager: StateManager,
    pub timeout_duration: Duration,
    pub prepared_packages: Arc<DashMap<PackageId, PreparedPackage>>,
    pub permit: OwnedSemaphorePermit,
}

/// Process a single package (download/local)
///
/// Handles both download and local package processing with comprehensive error handling.
/// Function length is due to two distinct workflows (download vs local) with event emission.
#[allow(clippy::too_many_lines)]
pub(crate) async fn process_package(args: ProcessPackageArgs) -> Result<PackageId, Error> {
    let ProcessPackageArgs {
        package_id,
        node,
        context,
        store,
        state_manager,
        timeout_duration,
        prepared_packages,
        permit: _permit,
    } = args;
    context.emit(AppEvent::General(GeneralEvent::DebugLog {
        message: format!(
            "DEBUG: Processing package {}-{} with action {:?}",
            package_id.name, package_id.version, node.action
        ),
        context: std::collections::HashMap::new(),
    }));

    context.emit(AppEvent::Install(InstallEvent::Started {
        package: package_id.name.clone(),
        version: package_id.version.clone(),
    }));

    match node.action {
        NodeAction::Download => {
            if let Some(url) = &node.url {
                // Download package with timeout and add to store (no validation)
                let download_result = tokio::time::timeout(
                    timeout_duration,
                    download_package_only(
                        url,
                        &package_id,
                        &node,
                        &store,
                        &state_manager,
                        &context,
                        &prepared_packages,
                    ),
                )
                .await;

                match download_result {
                    Ok(Ok(size)) => {
                        context.emit(AppEvent::Acquisition(AcquisitionEvent::Completed {
                            package: package_id.name.clone(),
                            version: package_id.version.clone(),
                            source: AcquisitionSource::Remote {
                                url: url.clone(),
                                mirror_priority: 0,
                            },
                            size,
                        }));
                    }
                    Ok(Err(e)) => {
                        let failure = FailureContext::from_error(&e);
                        context.emit(AppEvent::Acquisition(AcquisitionEvent::Failed {
                            package: package_id.name.clone(),
                            version: package_id.version.clone(),
                            source: AcquisitionSource::Remote {
                                url: url.clone(),
                                mirror_priority: 0,
                            },
                            failure,
                        }));
                        return Err(e);
                    }
                    Err(_) => {
                        let err: Error = InstallError::DownloadTimeout {
                            package: package_id.name.clone(),
                            url: url.to_string(),
                            timeout_seconds: timeout_duration.as_secs(),
                        }
                        .into();
                        let failure = FailureContext::from_error(&err);
                        context.emit(AppEvent::Acquisition(AcquisitionEvent::Failed {
                            package: package_id.name.clone(),
                            version: package_id.version.clone(),
                            source: AcquisitionSource::Remote {
                                url: url.clone(),
                                mirror_priority: 0,
                            },
                            failure,
                        }));
                        return Err(err);
                    }
                }
            } else {
                return Err(InstallError::MissingDownloadUrl {
                    package: package_id.name.clone(),
                }
                .into());
            }
        }
        NodeAction::Local => {
            context.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: format!(
                    "DEBUG: Processing local package {}-{}, path: {:?}",
                    package_id.name, package_id.version, node.path
                ),
                context: std::collections::HashMap::new(),
            }));

            if let Some(path) = &node.path {
                // Check if this is an already installed package (empty path)
                if path.as_os_str().is_empty() {
                    context.emit(AppEvent::General(GeneralEvent::DebugLog {
                        message: format!(
                            "DEBUG: Package {}-{} is already installed, skipping",
                            package_id.name, package_id.version
                        ),
                        context: std::collections::HashMap::new(),
                    }));

                    // For already installed packages, just mark as completed
                    context.emit(AppEvent::Install(InstallEvent::Completed {
                        package: package_id.name.clone(),
                        version: package_id.version.clone(),
                        files_installed: 0,
                    }));

                    return Ok(package_id);
                }
                context.emit(AppEvent::General(GeneralEvent::DebugLog {
                    message: format!("DEBUG: Adding local package to store: {}", path.display()),
                    context: std::collections::HashMap::new(),
                }));

                // For local packages, add to store and prepare data
                let stored_package = store.add_package(path).await?;

                if let Some(hash) = stored_package.hash() {
                    let size = stored_package.size().await?;
                    let store_path = stored_package.path().to_path_buf();

                    context.emit(AppEvent::General(GeneralEvent::DebugLog {
                        message: format!(
                            "DEBUG: Local package stored with hash {} at {}",
                            hash.to_hex(),
                            store_path.display()
                        ),
                        context: std::collections::HashMap::new(),
                    }));

                    let prepared_package = PreparedPackage {
                        hash: hash.clone(),
                        size,
                        store_path,
                        is_local: true,
                        package_hash: None,
                    };

                    prepared_packages.insert(package_id.clone(), prepared_package);

                    context.emit(AppEvent::General(GeneralEvent::DebugLog {
                        message: format!(
                            "DEBUG: Added prepared package for {}-{}",
                            package_id.name, package_id.version
                        ),
                        context: std::collections::HashMap::new(),
                    }));

                    context.emit(AppEvent::Install(InstallEvent::Completed {
                        package: package_id.name.clone(),
                        version: package_id.version.clone(),
                        files_installed: 0, // TODO: Count actual files
                    }));
                } else {
                    return Err(InstallError::AtomicOperationFailed {
                        message: "failed to get hash from local package".to_string(),
                    }
                    .into());
                }
            } else {
                return Err(InstallError::MissingLocalPath {
                    package: package_id.name.clone(),
                }
                .into());
            }
        }
    }

    Ok(package_id)
}

/// Download a package and add to store (no validation - `AtomicInstaller` handles that)
///
/// Complex download workflow including store caching, signature verification, and deduplication.
/// Function length reflects the comprehensive error handling and security checks required.
#[allow(clippy::too_many_lines)]
pub(crate) async fn download_package_only(
    url: &str,
    package_id: &PackageId,
    node: &ResolvedNode,
    store: &PackageStore,
    state_manager: &StateManager,
    context: &ExecutionContext,
    prepared_packages: &Arc<DashMap<PackageId, PreparedPackage>>,
) -> Result<u64, Error> {
    if let Some(size) = try_prepare_from_store(
        package_id,
        node,
        store,
        state_manager,
        context,
        prepared_packages,
    )
    .await?
    {
        return Ok(size);
    }

    context.emit(AppEvent::Acquisition(AcquisitionEvent::Started {
        package: package_id.name.clone(),
        version: package_id.version.clone(),
        source: AcquisitionSource::Remote {
            url: url.to_string(),
            mirror_priority: 0,
        },
    }));

    // Create a temporary directory for the download
    let temp_dir = tempfile::tempdir().map_err(|e| InstallError::TempFileError {
        message: e.to_string(),
    })?;

    // Use high-level PackageDownloader to benefit from hash/signature handling
    let downloader = PackageDownloader::new(
        PackageDownloadConfig::default(),
        sps2_events::ProgressManager::new(),
    )?;

    let tx = context
        .event_sender()
        .cloned()
        .unwrap_or_else(|| sps2_events::channel().0);

    let download_result = downloader
        .download_package(
            &package_id.name,
            &package_id.version,
            url,
            node.signature_url.as_deref(),
            temp_dir.path(),
            node.expected_hash.as_ref(),
            String::new(), // internal tracker
            None,
            &tx,
        )
        .await?;

    // Enforce signature policy if configured
    if let Some(policy) = context.security_policy() {
        if policy.verify_signatures && !policy.allow_unsigned {
            // If a signature was expected (URL provided), require verification
            if node.signature_url.is_some() {
                if !download_result.signature_verified {
                    return Err(sps2_errors::Error::Signing(
                        sps2_errors::SigningError::VerificationFailed {
                            reason: "package signature could not be verified".to_string(),
                        },
                    ));
                }
            } else {
                return Err(sps2_errors::Error::Signing(
                    sps2_errors::SigningError::InvalidSignatureFormat(
                        "missing signature for package".to_string(),
                    ),
                ));
            }
        }
    }

    let previous_store_hash = if context.force_redownload() {
        if let Some(expected_hash) = node.expected_hash.as_ref() {
            state_manager
                .get_store_hash_for_package_hash(&expected_hash.to_hex())
                .await?
                .map(|store_hash_hex| sps2_hash::Hash::from_hex(&store_hash_hex))
                .transpose()?
        } else {
            None
        }
    } else {
        None
    };

    // Add to store and prepare package data
    let mut stored_package = store
        .add_package_from_file(
            &download_result.package_path,
            &package_id.name,
            &package_id.version,
        )
        .await?;

    if let Some(prev_hash) = previous_store_hash {
        if let Some(current_hash) = stored_package.hash() {
            if current_hash == prev_hash {
                store.remove_package(&prev_hash).await?;
                stored_package = store
                    .add_package_from_file(
                        &download_result.package_path,
                        &package_id.name,
                        &package_id.version,
                    )
                    .await?;
            } else {
                store.remove_package(&prev_hash).await?;
            }
        }
    }

    if let Some(hash) = stored_package.hash() {
        let size = stored_package.size().await?;
        let store_path = stored_package.path().to_path_buf();

        let prepared_package = PreparedPackage {
            hash: hash.clone(),
            size,
            store_path,
            is_local: false,
            package_hash: node.expected_hash.clone(),
        };

        prepared_packages.insert(package_id.clone(), prepared_package);

        context.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "Package {}-{} downloaded and stored with hash {} (prepared for installation)",
                package_id.name,
                package_id.version,
                hash.to_hex()
            ),
            context: std::collections::HashMap::new(),
        }));
        Ok(size)
    } else {
        Err(InstallError::AtomicOperationFailed {
            message: "failed to get hash from downloaded package".to_string(),
        }
        .into())
    }
}

pub(crate) async fn try_prepare_from_store(
    package_id: &PackageId,
    node: &ResolvedNode,
    store: &PackageStore,
    state_manager: &StateManager,
    context: &ExecutionContext,
    prepared_packages: &Arc<DashMap<PackageId, PreparedPackage>>,
) -> Result<Option<u64>, Error> {
    if context.force_redownload() {
        return Ok(None);
    }

    let Some(expected_hash) = node.expected_hash.as_ref() else {
        return Ok(None);
    };

    let Some(store_hash_hex) = state_manager
        .get_store_hash_for_package_hash(&expected_hash.to_hex())
        .await?
    else {
        return Ok(None);
    };

    let store_hash = sps2_hash::Hash::from_hex(&store_hash_hex)?;

    let Some(stored_package) = store.load_package_if_exists(&store_hash).await? else {
        return Ok(None);
    };

    context.emit(AppEvent::Acquisition(AcquisitionEvent::Started {
        package: package_id.name.clone(),
        version: package_id.version.clone(),
        source: AcquisitionSource::StoreCache {
            hash: expected_hash.to_hex(),
        },
    }));

    let size = stored_package.size().await?;
    let store_path = stored_package.path().to_path_buf();

    let prepared_package = PreparedPackage {
        hash: store_hash,
        size,
        store_path,
        is_local: false,
        package_hash: Some(expected_hash.clone()),
    };

    prepared_packages.insert(package_id.clone(), prepared_package);

    context.emit(AppEvent::General(GeneralEvent::DebugLog {
        message: format!(
            "Reusing stored package {}-{} with hash {}",
            package_id.name,
            package_id.version,
            expected_hash.to_hex()
        ),
        context: std::collections::HashMap::new(),
    }));

    context.emit(AppEvent::Acquisition(AcquisitionEvent::Completed {
        package: package_id.name.clone(),
        version: package_id.version.clone(),
        source: AcquisitionSource::StoreCache {
            hash: expected_hash.to_hex(),
        },
        size,
    }));

    Ok(Some(size))
}

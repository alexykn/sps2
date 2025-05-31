//! Small operations implemented in the ops crate

use crate::{
    keys::KeyManager, ChangeType, ComponentHealth, HealthCheck, HealthIssue, HealthStatus,
    IssueSeverity, OpChange, OpsCtx, PackageInfo, PackageStatus, SearchResult, StateInfo,
    VulnDbStats,
};
use sps2_errors::{Error, OpsError};
use sps2_events::Event;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use uuid::Uuid;

/// Sync repository index
///
/// # Errors
///
/// Returns an error if index synchronization fails.
pub async fn reposync(ctx: &OpsCtx) -> Result<String, Error> {
    let start = Instant::now();

    ctx.tx.send(Event::RepoSyncStarting).ok();

    // Check if index is stale (older than 7 days)
    let stale = ctx.index.is_stale(7);

    if !stale {
        let message = "Repository index is up to date".to_string();
        ctx.tx
            .send(Event::RepoSyncCompleted {
                packages_updated: 0,
                duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            })
            .ok();
        return Ok(message);
    }

    // Repository URL (in real implementation, this would come from config)
    let base_url = "https://cdn.sps.io";
    let index_url = format!("{base_url}/index.json");
    let index_sig_url = format!("{base_url}/index.json.minisig");
    let keys_url = format!("{base_url}/keys.json");

    ctx.tx
        .send(Event::RepoSyncStarted {
            url: base_url.to_string(),
        })
        .ok();

    // 1. Download latest index.json and signature with `ETag` support
    let cached_etag = ctx.index.cache.load_etag().await.unwrap_or(None);

    let index_json =
        download_index_conditional(ctx, &index_url, cached_etag.as_deref(), start).await?;

    let index_signature = sps2_net::fetch_text(&ctx.net, &index_sig_url, &ctx.tx)
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to download index.json.minisig: {e}"),
        })?;

    // 2. Fetch and verify signing keys (with rotation support)
    let trusted_keys = fetch_and_verify_keys(&ctx.net, &keys_url, &ctx.tx)
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to verify signing keys: {e}"),
        })?;

    // 3. Verify signature of index.json
    verify_index_signature(&index_json, &index_signature, &trusted_keys).map_err(|e| {
        OpsError::RepoSyncFailed {
            message: format!("Index signature verification failed: {e}"),
        }
    })?;

    // Process and save the new index
    finalize_index_update(ctx, &index_json, start).await
}

/// Download index conditionally with `ETag` support
async fn download_index_conditional(
    ctx: &OpsCtx,
    index_url: &str,
    cached_etag: Option<&str>,
    start: Instant,
) -> Result<String, Error> {
    let response = sps2_net::fetch_text_conditional(&ctx.net, index_url, cached_etag, &ctx.tx)
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to download index.json: {e}"),
        })?;

    if let Some((content, new_etag)) = response {
        // Save new `ETag` if present
        if let Some(etag) = new_etag {
            if let Err(e) = ctx.index.cache.save_etag(&etag).await {
                // Log but don't fail the operation
                ctx.tx
                    .send(Event::Warning {
                        message: format!("Failed to save ETag: {e}"),
                        context: Some("ETag caching disabled for this session".to_string()),
                    })
                    .ok();
            }
        }
        Ok(content)
    } else {
        // Server returned 304 Not Modified - use cached content
        ctx.tx
            .send(Event::RepoSyncCompleted {
                packages_updated: 0,
                duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            })
            .ok();
        Err(OpsError::RepoSyncFailed {
            message: "Repository index is unchanged (304 Not Modified)".to_string(),
        }
        .into())
    }
}

/// Process and save the new index
async fn finalize_index_update(
    ctx: &OpsCtx,
    index_json: &str,
    start: Instant,
) -> Result<String, Error> {
    // Parse the new index to count changes
    let old_package_count = ctx.index.index().map_or(0, |idx| idx.packages.len());

    // Load the new index into IndexManager
    let mut new_index_manager = ctx.index.clone();
    new_index_manager
        .load(Some(index_json))
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to parse new index: {e}"),
        })?;

    let new_package_count = new_index_manager
        .index()
        .map_or(0, |idx| idx.packages.len());
    let packages_updated = new_package_count.saturating_sub(old_package_count);

    // Save to cache
    new_index_manager
        .save_to_cache()
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to save index cache: {e}"),
        })?;

    let message = if packages_updated > 0 {
        format!("Updated {packages_updated} packages from repository")
    } else {
        "Repository index updated (no new packages)".to_string()
    };

    ctx.tx
        .send(Event::RepoSyncCompleted {
            packages_updated,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
        .ok();

    Ok(message)
}

/// List installed packages
///
/// # Errors
///
/// Returns an error if package listing fails.
pub async fn list_packages(ctx: &OpsCtx) -> Result<Vec<PackageInfo>, Error> {
    ctx.tx.send(Event::ListStarting).ok();

    // Get installed packages from state
    let installed_packages = ctx.state.get_installed_packages().await?;

    let mut package_infos = Vec::new();

    for package in installed_packages {
        // Get package details from index
        let package_version = package.version();
        let index_entry = ctx
            .index
            .get_version(&package.name, &package_version.to_string());

        let (description, homepage, license, dependencies) = if let Some(entry) = index_entry {
            (
                entry.description.clone(),
                entry.homepage.clone(),
                entry.license.clone(),
                entry.dependencies.runtime.clone(),
            )
        } else {
            (None, None, None, Vec::new())
        };

        // Check if there's an available update
        let available_version =
            ctx.index
                .get_package_versions(&package.name)
                .and_then(|versions| {
                    versions
                        .first()
                        .and_then(|entry| sps2_types::Version::parse(&entry.version()).ok())
                });

        let status = match &available_version {
            Some(avail) if avail > &package_version => PackageStatus::Outdated,
            _ => PackageStatus::Installed,
        };

        // Get package size from store
        let size = ctx
            .store
            .get_package_size(&package.name, &package_version)
            .ok();

        let package_info = PackageInfo {
            name: package.name.clone(),
            version: Some(package_version),
            available_version,
            description,
            homepage,
            license,
            status,
            dependencies,
            size,
        };

        package_infos.push(package_info);
    }

    // Sort by name
    package_infos.sort_by(|a, b| a.name.cmp(&b.name));

    ctx.tx
        .send(Event::ListCompleted {
            count: package_infos.len(),
        })
        .ok();

    Ok(package_infos)
}

/// Get information about a specific package
///
/// # Errors
///
/// Returns an error if package information retrieval fails.
pub async fn package_info(ctx: &OpsCtx, package_name: &str) -> Result<PackageInfo, Error> {
    // Check if package is installed
    let installed_packages = ctx.state.get_installed_packages().await?;
    let installed_version = installed_packages
        .iter()
        .find(|pkg| pkg.name == package_name)
        .map(sps2_state::Package::version);

    // Get available versions from index
    let versions = ctx
        .index
        .get_package_versions(package_name)
        .ok_or_else(|| OpsError::PackageNotFound {
            package: package_name.to_string(),
        })?;

    let latest_entry = versions.first().ok_or_else(|| OpsError::PackageNotFound {
        package: package_name.to_string(),
    })?;

    let available_version = sps2_types::Version::parse(&latest_entry.version())?;

    let status = match &installed_version {
        Some(installed) => {
            match installed.cmp(&available_version) {
                std::cmp::Ordering::Equal => PackageStatus::Installed,
                std::cmp::Ordering::Less => PackageStatus::Outdated,
                std::cmp::Ordering::Greater => PackageStatus::Local, // Newer than available
            }
        }
        None => PackageStatus::Available,
    };

    // Get package size if installed
    let size = if let Some(version) = &installed_version {
        ctx.store.get_package_size(package_name, version).ok()
    } else {
        None
    };

    let package_info = PackageInfo {
        name: package_name.to_string(),
        version: installed_version,
        available_version: Some(available_version),
        description: latest_entry.description.clone(),
        homepage: latest_entry.homepage.clone(),
        license: latest_entry.license.clone(),
        status,
        dependencies: latest_entry.dependencies.runtime.clone(),
        size,
    };

    Ok(package_info)
}

/// Search for packages
///
/// # Errors
///
/// Returns an error if package search fails.
pub async fn search_packages(ctx: &OpsCtx, query: &str) -> Result<Vec<SearchResult>, Error> {
    ctx.tx
        .send(Event::SearchStarting {
            query: query.to_string(),
        })
        .ok();

    // Search package names in index
    let package_names = ctx.index.search(query);

    let mut results = Vec::new();
    let installed_packages = ctx.state.get_installed_packages().await?;

    for package_name in package_names {
        if let Some(versions) = ctx.index.get_package_versions(package_name) {
            if let Some(latest) = versions.first() {
                if let Ok(version) = sps2_types::Version::parse(&latest.version()) {
                    let installed = installed_packages
                        .iter()
                        .any(|pkg| pkg.name == package_name);

                    results.push(SearchResult {
                        name: package_name.to_string(),
                        version,
                        description: latest.description.clone(),
                        installed,
                    });
                }
            }
        }
    }

    ctx.tx
        .send(Event::SearchCompleted {
            query: query.to_string(),
            count: results.len(),
        })
        .ok();

    Ok(results)
}

/// Clean up orphaned packages and old states
///
/// # Errors
///
/// Returns an error if cleanup operation fails.
pub async fn cleanup(ctx: &OpsCtx) -> Result<String, Error> {
    let start = Instant::now();

    ctx.tx.send(Event::CleanupStarting).ok();

    // Clean up old states (keep last 10)
    let cleaned_states = ctx.state.cleanup_old_states(10).await?;

    // Run garbage collection on store
    let cleaned_packages = ctx.state.gc_store_with_removal(&ctx.store).await?;

    let duration = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let message = format!(
        "Cleaned up {} old states and {} orphaned packages",
        cleaned_states.len(),
        cleaned_packages
    );

    ctx.tx
        .send(Event::CleanupCompleted {
            states_removed: cleaned_states.len(),
            packages_removed: cleaned_packages,
            duration_ms: duration,
        })
        .ok();

    // Update GC timestamp after successful cleanup
    if let Err(e) = update_gc_timestamp().await {
        // Log but don't fail the cleanup operation
        eprintln!("Warning: Failed to update GC timestamp: {e}");
    }

    Ok(message)
}

/// Rollback to a previous state
///
/// # Errors
///
/// Returns an error if:
/// - No previous state exists
/// - Rollback operation fails
pub async fn rollback(ctx: &OpsCtx, target_state: Option<Uuid>) -> Result<StateInfo, Error> {
    let start = Instant::now();

    // If no target specified, rollback to previous state
    let target_id = if let Some(id) = target_state {
        id
    } else {
        let current_id = ctx.state.get_current_state_id().await?;

        ctx.state
            .get_parent_state_id(&current_id)
            .await?
            .ok_or(OpsError::NoPreviousState)?
    };

    ctx.tx
        .send(Event::RollbackStarting {
            target_state: target_id,
        })
        .ok();

    // Verify target state exists
    if !ctx.state.state_exists(&target_id).await? {
        return Err(OpsError::StateNotFound {
            state_id: target_id,
        }
        .into());
    }

    // Perform rollback using atomic installer
    let mut atomic_installer =
        sps2_install::AtomicInstaller::new(ctx.state.clone(), ctx.store.clone());

    atomic_installer.rollback(target_id).await?;

    // Get state information
    let state_info = get_state_info(ctx, target_id).await?;

    ctx.tx
        .send(Event::RollbackCompleted {
            target_state: target_id,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
        .ok();

    Ok(state_info)
}

/// Get history of states
///
/// # Errors
///
/// Returns an error if state history retrieval fails.
pub async fn history(ctx: &OpsCtx) -> Result<Vec<StateInfo>, Error> {
    let states = ctx.state.list_states_detailed().await?;
    let current_id = ctx.state.get_current_state_id().await?;

    let mut state_infos = Vec::new();

    for state in states {
        let state_id = state.state_id();
        let parent_id = state
            .parent_id
            .as_ref()
            .and_then(|p| uuid::Uuid::parse_str(p).ok());

        // Get actual package count for this state
        let package_count = get_state_package_count(ctx, &state_id).await?;

        // Calculate changes from parent state
        let changes = if let Some(parent_id) = parent_id {
            calculate_state_changes(ctx, &parent_id, &state_id).await?
        } else {
            // Root state - all packages are installs
            get_initial_state_changes(ctx, &state_id).await?
        };

        let state_info = StateInfo {
            id: state_id,
            timestamp: state.timestamp(),
            parent_id,
            current: Some(current_id) == Some(state_id),
            package_count,
            changes,
        };

        state_infos.push(state_info);
    }

    // Sort by timestamp (newest first)
    state_infos.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(state_infos)
}

/// Check system health
///
/// # Errors
///
/// Returns an error if health check fails.
pub async fn check_health(ctx: &OpsCtx) -> Result<HealthCheck, Error> {
    let _start = Instant::now();

    ctx.tx.send(Event::HealthCheckStarting).ok();

    let mut components = HashMap::new();
    let mut issues = Vec::new();
    let mut overall_healthy = true;

    // Check store health
    let store_start = Instant::now();
    let store_health = check_store_health(ctx, &mut issues);
    components.insert(
        "store".to_string(),
        ComponentHealth {
            name: "Store".to_string(),
            status: store_health,
            message: "Package store integrity check".to_string(),
            check_duration_ms: u64::try_from(store_start.elapsed().as_millis()).unwrap_or(u64::MAX),
        },
    );

    if !matches!(store_health, HealthStatus::Healthy) {
        overall_healthy = false;
    }

    // Check state database health
    let state_start = Instant::now();
    let state_health = check_state_health(ctx, &mut issues).await;
    components.insert(
        "state".to_string(),
        ComponentHealth {
            name: "State Database".to_string(),
            status: state_health,
            message: "State database consistency check".to_string(),
            check_duration_ms: u64::try_from(state_start.elapsed().as_millis()).unwrap_or(u64::MAX),
        },
    );

    if !matches!(state_health, HealthStatus::Healthy) {
        overall_healthy = false;
    }

    // Check index health
    let index_start = Instant::now();
    let index_health = check_index_health(ctx, &mut issues);
    components.insert(
        "index".to_string(),
        ComponentHealth {
            name: "Package Index".to_string(),
            status: index_health,
            message: "Package index freshness check".to_string(),
            check_duration_ms: u64::try_from(index_start.elapsed().as_millis()).unwrap_or(u64::MAX),
        },
    );

    if !matches!(index_health, HealthStatus::Healthy) {
        overall_healthy = false;
    }

    let health_check = HealthCheck {
        healthy: overall_healthy,
        components,
        issues,
    };

    ctx.tx
        .send(Event::HealthCheckCompleted {
            healthy: overall_healthy,
            issues: health_check
                .issues
                .iter()
                .map(|i| i.description.clone())
                .collect(),
        })
        .ok();

    Ok(health_check)
}

/// Check store health
fn check_store_health(ctx: &OpsCtx, issues: &mut Vec<HealthIssue>) -> HealthStatus {
    // Check if store directory exists and is accessible
    if ctx.store.verify_integrity().is_ok() {
        HealthStatus::Healthy
    } else {
        issues.push(HealthIssue {
            component: "store".to_string(),
            severity: IssueSeverity::High,
            description: "Package store integrity check failed".to_string(),
            suggestion: Some("Run 'sps2 cleanup' to fix corrupted store entries".to_string()),
        });
        HealthStatus::Error
    }
}

/// Check state database health
async fn check_state_health(ctx: &OpsCtx, issues: &mut Vec<HealthIssue>) -> HealthStatus {
    // Check database consistency
    if ctx.state.verify_consistency().await.is_ok() {
        HealthStatus::Healthy
    } else {
        issues.push(HealthIssue {
            component: "state".to_string(),
            severity: IssueSeverity::Critical,
            description: "State database consistency check failed".to_string(),
            suggestion: Some(
                "Database may be corrupted, consider restoring from backup".to_string(),
            ),
        });
        HealthStatus::Error
    }
}

/// Check index health
fn check_index_health(ctx: &OpsCtx, issues: &mut Vec<HealthIssue>) -> HealthStatus {
    // Check if index is stale
    if ctx.index.is_stale(7) {
        issues.push(HealthIssue {
            component: "index".to_string(),
            severity: IssueSeverity::Medium,
            description: "Package index is outdated (>7 days old)".to_string(),
            suggestion: Some("Run 'sps2 reposync' to update package index".to_string()),
        });
        HealthStatus::Warning
    } else {
        HealthStatus::Healthy
    }
}

/// Get state information by ID
async fn get_state_info(ctx: &OpsCtx, state_id: Uuid) -> Result<StateInfo, Error> {
    let states = ctx.state.list_states_detailed().await?;
    let current_id = ctx.state.get_current_state_id().await?;

    let state = states
        .iter()
        .find(|s| s.state_id() == state_id)
        .ok_or(OpsError::StateNotFound { state_id })?;

    let parent_id = state
        .parent_id
        .as_ref()
        .and_then(|p| uuid::Uuid::parse_str(p).ok());

    // Get actual package count for this state
    let package_count = get_state_package_count(ctx, &state_id).await?;

    // Calculate changes from parent state
    let changes = if let Some(parent_id) = parent_id {
        calculate_state_changes(ctx, &parent_id, &state_id).await?
    } else {
        // Root state - all packages are installs
        get_initial_state_changes(ctx, &state_id).await?
    };

    Ok(StateInfo {
        id: state_id,
        timestamp: state.timestamp(),
        parent_id,
        current: Some(current_id) == Some(state_id),
        package_count,
        changes,
    })
}

/// Update vulnerability database
///
/// # Errors
///
/// Returns an error if the vulnerability database update fails.
pub async fn update_vulndb(_ctx: &OpsCtx) -> Result<String, Error> {
    // Initialize vulnerability database manager
    let mut vulndb = sps2_audit::VulnDbManager::new(sps2_audit::VulnDbManager::default_path())?;

    // Initialize if needed
    vulndb.initialize().await?;

    // Update the database from all sources
    vulndb.update().await?;

    Ok("Vulnerability database updated successfully".to_string())
}

/// Get vulnerability database statistics
///
/// # Errors
///
/// Returns an error if the vulnerability database cannot be accessed.
pub async fn vulndb_stats(_ctx: &OpsCtx) -> Result<VulnDbStats, Error> {
    // Initialize vulnerability database manager
    let mut vulndb = sps2_audit::VulnDbManager::new(sps2_audit::VulnDbManager::default_path())?;

    // Initialize if needed
    vulndb.initialize().await?;

    // Get database
    let db = vulndb.get_database().await?;

    // Get statistics
    let stats = db.get_statistics().await?;

    // Get database file size
    let db_path = sps2_audit::VulnDbManager::default_path();
    let metadata = tokio::fs::metadata(&db_path).await?;
    let database_size = metadata.len();

    Ok(VulnDbStats {
        vulnerability_count: stats.vulnerability_count,
        last_updated: stats.last_updated,
        database_size,
        severity_breakdown: stats.severity_breakdown,
    })
}

/// Audit packages for vulnerabilities
///
/// # Errors
///
/// Returns an error if the audit scan fails.
pub async fn audit(
    ctx: &OpsCtx,
    package_name: Option<&str>,
    fail_on_critical: bool,
    severity_threshold: sps2_audit::Severity,
) -> Result<sps2_audit::AuditReport, Error> {
    // Create audit system
    let audit_system = sps2_audit::AuditSystem::new(sps2_audit::VulnDbManager::default_path())?;

    // Configure scan options
    let scan_options = sps2_audit::ScanOptions::new()
        .with_fail_on_critical(fail_on_critical)
        .with_severity_threshold(severity_threshold);

    // Run audit based on whether a specific package is requested
    let report = if let Some(name) = package_name {
        // Scan specific package
        let installed_packages = ctx.state.get_installed_packages().await?;
        let package = installed_packages
            .iter()
            .find(|pkg| pkg.name == name)
            .ok_or_else(|| OpsError::PackageNotFound {
                package: name.to_string(),
            })?;

        ctx.tx.send(Event::AuditStarting { package_count: 1 }).ok();

        let package_audit = audit_system
            .scan_package(&package.name, &package.version(), &ctx.store, &scan_options)
            .await?;

        let vuln_count = package_audit.vulnerabilities.len();
        ctx.tx
            .send(Event::AuditPackageCompleted {
                package: package.name.clone(),
                vulnerabilities_found: vuln_count,
            })
            .ok();

        let report = sps2_audit::AuditReport::new(vec![package_audit]);

        ctx.tx
            .send(Event::AuditCompleted {
                packages_scanned: 1,
                vulnerabilities_found: report.total_vulnerabilities(),
                critical_count: report.critical_count(),
            })
            .ok();

        report
    } else {
        // Scan all packages
        audit_system
            .scan_all_packages(&ctx.state, &ctx.store, scan_options, Some(ctx.tx.clone()))
            .await?
    };

    Ok(report)
}

/// Fetch and verify signing keys with rotation support
async fn fetch_and_verify_keys(
    net_client: &sps2_net::NetClient,
    keys_url: &str,
    tx: &sps2_events::EventSender,
) -> Result<Vec<String>, Error> {
    // Initialize key manager
    let mut key_manager = KeyManager::new("/opt/pm/keys");

    // Load existing trusted keys from disk
    key_manager.load_trusted_keys().await?;

    // Check if we have any trusted keys; if not, initialize with bootstrap
    if key_manager.get_trusted_keys().is_empty() {
        // Bootstrap key for initial trust - in production this would be:
        // 1. Compiled into the binary
        // 2. Distributed through secure channels
        // 3. Verified through multiple sources
        let bootstrap_key = "RWSGOq2NVecA2UPNdBUZykp1MLhfMmkAK/SZSjK3bpq2q7I8LbSVVBDm";

        tx.send(Event::Warning {
            message: "Initializing with bootstrap key".to_string(),
            context: Some("First run - no trusted keys found".to_string()),
        })
        .ok();

        key_manager
            .initialize_with_bootstrap(bootstrap_key)
            .await
            .map_err(|e| OpsError::RepoSyncFailed {
                message: format!("Failed to initialize bootstrap key: {e}"),
            })?;
    }

    // Fetch and verify keys from repository
    let trusted_keys = key_manager
        .fetch_and_verify_keys(net_client, keys_url, tx)
        .await?;

    tx.send(Event::OperationCompleted {
        operation: "Key verification".to_string(),
        success: true,
    })
    .ok();

    Ok(trusted_keys)
}

/// Verify minisign signature of index.json
fn verify_index_signature(
    index_content: &str,
    signature: &str,
    trusted_keys: &[String],
) -> Result<(), Error> {
    if index_content.is_empty() {
        return Err(OpsError::RepoSyncFailed {
            message: "Index content is empty".to_string(),
        }
        .into());
    }

    if signature.is_empty() {
        return Err(OpsError::RepoSyncFailed {
            message: "Signature is empty".to_string(),
        }
        .into());
    }

    if trusted_keys.is_empty() {
        return Err(OpsError::RepoSyncFailed {
            message: "No trusted keys available for verification".to_string(),
        }
        .into());
    }

    // Parse the minisign signature - expect format:
    // untrusted comment: <comment>
    // <base64-signature>
    let signature_lines: Vec<&str> = signature.lines().collect();
    if signature_lines.len() < 2 {
        return Err(OpsError::RepoSyncFailed {
            message: "Invalid minisign signature format - missing lines".to_string(),
        }
        .into());
    }

    if !signature_lines[0].starts_with("untrusted comment:") {
        return Err(OpsError::RepoSyncFailed {
            message: "Invalid minisign signature format - missing comment line".to_string(),
        }
        .into());
    }

    // Use the full signature content (not just the base64 part)
    let sig =
        minisign_verify::Signature::decode(signature).map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to parse signature: {e}"),
        })?;

    // Try verification with each trusted key until one succeeds
    let mut verification_errors = Vec::new();

    for trusted_key in trusted_keys {
        match minisign_verify::PublicKey::from_base64(trusted_key) {
            Ok(public_key) => {
                // Try to verify with this key - the verify method handles key ID comparison internally
                match public_key.verify(index_content.as_bytes(), &sig, false) {
                    Ok(()) => {
                        // Signature verification successful
                        return Ok(());
                    }
                    Err(e) => {
                        verification_errors.push(format!("Key verification failed: {e}"));
                    }
                }
            }
            Err(e) => {
                verification_errors.push(format!("Invalid trusted key format: {e}"));
            }
        }
    }

    // If we get here, no key successfully verified the signature
    Err(OpsError::RepoSyncFailed {
        message: format!(
            "Index signature verification failed. Tried {} trusted keys. Errors: {}",
            trusted_keys.len(),
            verification_errors.join("; ")
        ),
    }
    .into())
}

/// Get package count for a specific state
async fn get_state_package_count(ctx: &OpsCtx, state_id: &Uuid) -> Result<usize, Error> {
    let packages = ctx.state.get_state_packages(state_id).await?;
    Ok(packages.len())
}

/// Calculate changes between parent and child states
async fn calculate_state_changes(
    ctx: &OpsCtx,
    parent_id: &Uuid,
    child_id: &Uuid,
) -> Result<Vec<OpChange>, Error> {
    let parent_packages = ctx.state.get_state_packages(parent_id).await?;
    let child_packages = ctx.state.get_state_packages(child_id).await?;

    let mut changes = Vec::new();

    // Convert to sets for easier comparison
    let parent_set: std::collections::HashSet<&String> = parent_packages.iter().collect();
    let child_set: std::collections::HashSet<&String> = child_packages.iter().collect();

    // Find packages that were added (in child but not parent)
    for package in &child_packages {
        if !parent_set.contains(package) {
            // For now, we can't get version info from package names only
            // In a real implementation, we'd need to get full Package objects
            changes.push(OpChange {
                change_type: ChangeType::Install,
                package: package.clone(),
                old_version: None,
                new_version: None, // Would need actual Package data
            });
        }
    }

    // Find packages that were removed (in parent but not child)
    for package in &parent_packages {
        if !child_set.contains(package) {
            changes.push(OpChange {
                change_type: ChangeType::Remove,
                package: package.clone(),
                old_version: None, // Would need actual Package data
                new_version: None,
            });
        }
    }

    // Note: Updates/downgrades would require version comparison
    // which needs full Package objects, not just names

    Ok(changes)
}

/// Get changes for initial state (all packages are installs)
async fn get_initial_state_changes(ctx: &OpsCtx, state_id: &Uuid) -> Result<Vec<OpChange>, Error> {
    let packages = ctx.state.get_state_packages(state_id).await?;
    let mut changes = Vec::new();

    for package in packages {
        changes.push(OpChange {
            change_type: ChangeType::Install,
            package,
            old_version: None,
            new_version: None, // Would need actual Package data
        });
    }

    Ok(changes)
}

/// Update the GC timestamp after successful cleanup
async fn update_gc_timestamp() -> Result<(), Error> {
    let timestamp_path = std::path::Path::new("/opt/pm/.last_gc_timestamp");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    tokio::fs::write(timestamp_path, now.to_string())
        .await
        .map_err(|e| sps2_errors::Error::internal(format!("Failed to write GC timestamp: {e}")))?;

    Ok(())
}

/// Update sps2 to the latest version
///
/// # Errors
///
/// Returns an error if:
/// - Failed to check for latest version
/// - Failed to download or verify the new binary
/// - Failed to replace the current executable
pub async fn self_update(ctx: &OpsCtx, skip_verify: bool, force: bool) -> Result<String, Error> {
    let start = Instant::now();
    let current_version = env!("CARGO_PKG_VERSION");

    ctx.tx.send(Event::SelfUpdateStarting).ok();
    ctx.tx
        .send(Event::SelfUpdateCheckingVersion {
            current_version: current_version.to_string(),
        })
        .ok();

    // Check latest version from GitHub API
    let latest_version = get_latest_version(&ctx.net, &ctx.tx).await?;

    // Compare versions
    let current = sps2_types::Version::parse(current_version)?;
    let latest = sps2_types::Version::parse(&latest_version)?;

    if !force && latest <= current {
        ctx.tx
            .send(Event::SelfUpdateAlreadyLatest {
                version: current_version.to_string(),
            })
            .ok();
        return Ok(format!("Already on latest version: {current_version}"));
    }

    ctx.tx
        .send(Event::SelfUpdateVersionAvailable {
            current_version: current_version.to_string(),
            latest_version: latest_version.clone(),
        })
        .ok();

    // Determine download URLs for ARM64 macOS
    let binary_url = format!(
        "https://github.com/sps-io/sps2/releases/download/v{latest_version}/sps2-{latest_version}-aarch64-apple-darwin"
    );
    let signature_url = format!("{binary_url}.minisig");

    ctx.tx
        .send(Event::SelfUpdateDownloading {
            version: latest_version.clone(),
            url: binary_url.clone(),
        })
        .ok();

    // Create temporary directory for download
    let temp_dir = tempfile::tempdir().map_err(|e| OpsError::SelfUpdateFailed {
        message: format!("Failed to create temp directory: {e}"),
    })?;

    let temp_binary = temp_dir.path().join("sps2-new");
    let temp_signature = temp_dir.path().join("sps2-new.minisig");

    // Download new binary
    sps2_net::download_file(&ctx.net, &binary_url, &temp_binary, None, &ctx.tx)
        .await
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to download binary: {e}"),
        })?;

    if !skip_verify {
        ctx.tx
            .send(Event::SelfUpdateVerifying {
                version: latest_version.clone(),
            })
            .ok();

        // Download signature
        sps2_net::download_file(&ctx.net, &signature_url, &temp_signature, None, &ctx.tx)
            .await
            .map_err(|e| OpsError::SelfUpdateFailed {
                message: format!("Failed to download signature: {e}"),
            })?;

        // Verify signature
        verify_binary_signature(&temp_binary, &temp_signature).await?;
    }

    ctx.tx
        .send(Event::SelfUpdateInstalling {
            version: latest_version.clone(),
        })
        .ok();

    // Replace current executable atomically
    replace_current_executable(&temp_binary).await?;

    let duration = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    ctx.tx
        .send(Event::SelfUpdateCompleted {
            old_version: current_version.to_string(),
            new_version: latest_version.clone(),
            duration_ms: duration,
        })
        .ok();

    Ok(format!(
        "Updated from {current_version} to {latest_version}"
    ))
}

/// Get latest version from GitHub releases API
async fn get_latest_version(
    net_client: &sps2_net::NetClient,
    tx: &sps2_events::EventSender,
) -> Result<String, Error> {
    let api_url = "https://api.github.com/repos/sps-io/sps2/releases/latest";

    let response_text = sps2_net::fetch_text(net_client, api_url, tx)
        .await
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to fetch release info: {e}"),
        })?;

    let release: serde_json::Value =
        serde_json::from_str(&response_text).map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to parse release JSON: {e}"),
        })?;

    let tag_name = release["tag_name"]
        .as_str()
        .ok_or_else(|| OpsError::SelfUpdateFailed {
            message: "Release JSON missing tag_name field".to_string(),
        })?;

    // Remove 'v' prefix if present
    let version = tag_name.strip_prefix('v').unwrap_or(tag_name);
    Ok(version.to_string())
}

/// Verify binary signature using minisign
async fn verify_binary_signature(binary_path: &Path, signature_path: &Path) -> Result<(), Error> {
    let binary_content =
        tokio::fs::read(binary_path)
            .await
            .map_err(|e| OpsError::SelfUpdateFailed {
                message: format!("Failed to read binary for verification: {e}"),
            })?;

    let signature_content = tokio::fs::read_to_string(signature_path)
        .await
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to read signature: {e}"),
        })?;

    // Parse signature
    let signature = minisign_verify::Signature::decode(&signature_content).map_err(|e| {
        OpsError::SelfUpdateFailed {
            message: format!("Failed to parse signature: {e}"),
        }
    })?;

    // Use the same release signing key as for packages
    // In production, this would be the same trusted key used for package verification
    let trusted_key = "RWSGOq2NVecA2UPNdBUZykp1MLhfMmkAK/SZSjK3bpq2q7I8LbSVVBDm";

    let public_key = minisign_verify::PublicKey::from_base64(trusted_key).map_err(|e| {
        OpsError::SelfUpdateFailed {
            message: format!("Failed to parse public key: {e}"),
        }
    })?;

    public_key
        .verify(&binary_content, &signature, false)
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Binary signature verification failed: {e}"),
        })?;

    Ok(())
}

/// Replace current executable atomically
async fn replace_current_executable(new_binary_path: &Path) -> Result<(), Error> {
    // Get current executable path
    let current_exe = std::env::current_exe().map_err(|e| OpsError::SelfUpdateFailed {
        message: format!("Failed to get current executable path: {e}"),
    })?;

    // Make new binary executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(new_binary_path)
            .await
            .map_err(|e| OpsError::SelfUpdateFailed {
                message: format!("Failed to get binary metadata: {e}"),
            })?
            .permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(new_binary_path, perms)
            .await
            .map_err(|e| OpsError::SelfUpdateFailed {
                message: format!("Failed to set binary permissions: {e}"),
            })?;
    }

    // Create backup of current executable
    let backup_path = current_exe.with_extension("backup");
    tokio::fs::copy(&current_exe, &backup_path)
        .await
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to create backup: {e}"),
        })?;

    // Atomic replacement using rename
    tokio::fs::rename(new_binary_path, &current_exe)
        .await
        .map_err(|e| {
            // Attempt to restore backup on failure
            if let Err(restore_err) = std::fs::rename(&backup_path, &current_exe) {
                OpsError::SelfUpdateFailed {
                    message: format!(
                        "Failed to replace executable: {e}. Also failed to restore backup: {restore_err}"
                    ),
                }
            } else {
                OpsError::SelfUpdateFailed {
                    message: format!("Failed to replace executable: {e}. Restored from backup."),
                }
            }
        })?;

    // Clean up backup on success
    let _ = tokio::fs::remove_file(backup_path).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_index::Index;
    use tempfile::tempdir;

    async fn create_test_context() -> OpsCtx {
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        // Create necessary directories
        std::fs::create_dir_all(base_path.join("store")).unwrap();
        std::fs::create_dir_all(base_path.join("states")).unwrap();
        std::fs::create_dir_all(base_path.join("live")).unwrap();

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let store = sps2_store::PackageStore::new(base_path.join("store"));

        // Create StateManager with explicit error handling
        let state = match sps2_state::StateManager::new(base_path).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to create StateManager at {base_path:?}: {e}");
                eprintln!("Directory exists: {}", base_path.exists());
                eprintln!(
                    "Directory is writable: {}",
                    base_path
                        .metadata()
                        .map(|m| !m.permissions().readonly())
                        .unwrap_or(false)
                );
                panic!("StateManager creation failed: {e}");
            }
        };
        let mut index = sps2_index::IndexManager::new(base_path);
        let empty_index = Index::new();
        let json = empty_index.to_json().unwrap();
        index.load(Some(&json)).await.unwrap();

        let net = sps2_net::NetClient::with_defaults().unwrap();
        let resolver = sps2_resolver::Resolver::new(index.clone());
        let builder = sps2_builder::Builder::new().with_net(net.clone());

        OpsCtx::new(store, state, index, net, resolver, builder, tx)
    }

    #[tokio::test]
    async fn test_reposync() {
        let ctx = create_test_context().await;
        let result = reposync(&ctx).await.unwrap();
        assert!(result.contains("Repository index"));
    }

    #[tokio::test]
    async fn test_list_packages() {
        let ctx = create_test_context().await;

        // In a fresh system, there's no active state, so list_packages will fail
        let result = list_packages(&ctx).await;

        // For now, we expect this to fail with ActiveStateMissing
        assert!(result.is_err());

        // TODO: Once we have state creation in tests, update this test
    }

    #[tokio::test]
    async fn test_search_packages() {
        let ctx = create_test_context().await;

        // Search needs an active state to check installed packages
        let result = search_packages(&ctx, "test").await;

        // For now, we expect this to fail with ActiveStateMissing
        assert!(result.is_err());

        // TODO: Once we have state creation and a populated index in tests, update this test
    }

    #[tokio::test]
    async fn test_cleanup() {
        let ctx = create_test_context().await;

        // Cleanup also needs an active state
        let result = cleanup(&ctx).await;

        // For now, we expect this to fail with ActiveStateMissing
        assert!(result.is_err());

        // TODO: Once we have state creation in tests, update this test
    }

    #[tokio::test]
    async fn test_history() {
        let ctx = create_test_context().await;

        // History needs an active state to determine which is current
        let result = history(&ctx).await;

        // For now, we expect this to fail with ActiveStateMissing
        assert!(result.is_err());

        // TODO: Once we have state creation in tests, update this test
    }

    #[tokio::test]
    async fn test_check_health() {
        let ctx = create_test_context().await;
        let health = check_health(&ctx).await.unwrap();

        // Should have checks for store, state, and index
        assert!(health.components.contains_key("store"));
        assert!(health.components.contains_key("state"));
        assert!(health.components.contains_key("index"));
    }

    #[tokio::test]
    async fn test_audit() {
        let ctx = create_test_context().await;

        // Audit needs an active state to check installed packages
        let result = audit(&ctx, None, false, sps2_audit::Severity::Low).await;

        // For now, we expect this to fail with ActiveStateMissing
        assert!(result.is_err());

        // TODO: Once we have state creation in tests, update this test
    }
}

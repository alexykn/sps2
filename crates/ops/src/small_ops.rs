//! Small operations implemented in the ops crate

use crate::{
    ComponentHealth, HealthCheck, HealthIssue, HealthStatus, IssueSeverity, OpsCtx, PackageInfo,
    PackageStatus, SearchResult, StateInfo,
};
use spsv2_errors::{Error, OpsError};
use spsv2_events::Event;
use std::collections::HashMap;
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

    // TODO: In a real implementation, this would:
    // 1. Download latest index from repository
    // 2. Verify signatures
    // 3. Update local cache
    // 4. Count new/updated packages

    // For now, just simulate the operation
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let packages_updated = 42; // Simulated count
    let message = format!("Updated {packages_updated} packages from repository");

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
                        .and_then(|entry| spsv2_types::Version::parse(&entry.version()).ok())
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
        .map(spsv2_state::Package::version);

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

    let available_version = spsv2_types::Version::parse(&latest_entry.version())?;

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
                if let Ok(version) = spsv2_types::Version::parse(&latest.version()) {
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
    let cleaned_packages = ctx.store.garbage_collect()?;

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
        spsv2_install::AtomicInstaller::new(ctx.state.clone(), ctx.store.clone());

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

        let state_info = StateInfo {
            id: state_id,
            timestamp: state.timestamp(),
            parent_id,
            current: Some(current_id) == Some(state_id),
            package_count: 0,    // TODO: Get actual package count
            changes: Vec::new(), // TODO: Calculate changes from parent
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

    Ok(StateInfo {
        id: state_id,
        timestamp: state.timestamp(),
        parent_id,
        current: Some(current_id) == Some(state_id),
        package_count: 0,    // TODO: Get actual package count
        changes: Vec::new(), // TODO: Calculate changes from parent
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spsv2_index::Index;
    use tempfile::tempdir;

    async fn create_test_context() -> OpsCtx {
        let temp = tempdir().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let store = spsv2_store::PackageStore::new(temp.path().to_path_buf());
        let state = spsv2_state::StateManager::new(temp.path()).await.unwrap();
        let mut index = spsv2_index::IndexManager::new(temp.path());
        let empty_index = Index::new();
        let json = empty_index.to_json().unwrap();
        index.load(Some(&json)).await.unwrap();

        let net = spsv2_net::NetClient::with_defaults().unwrap();
        let resolver = spsv2_resolver::Resolver::new(index.clone());
        let builder = spsv2_builder::Builder::new();

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
        let packages = list_packages(&ctx).await.unwrap();
        assert!(packages.is_empty()); // No packages installed initially
    }

    #[tokio::test]
    async fn test_search_packages() {
        let ctx = create_test_context().await;
        let results = search_packages(&ctx, "test").await.unwrap();
        assert!(results.is_empty()); // No packages in empty index
    }

    #[tokio::test]
    async fn test_cleanup() {
        let ctx = create_test_context().await;
        let result = cleanup(&ctx).await.unwrap();
        assert!(result.contains("Cleaned up"));
    }

    #[tokio::test]
    async fn test_history() {
        let ctx = create_test_context().await;
        let history = history(&ctx).await.unwrap();
        // Should have at least the initial state
        assert!(!history.is_empty());
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
}

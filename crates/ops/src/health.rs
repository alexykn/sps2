//! System Health and Diagnostics Operations

use crate::{ComponentHealth, HealthCheck, HealthIssue, HealthStatus, IssueSeverity, OpsCtx};
use sps2_errors::Error;
use sps2_events::{AppEvent, EventEmitter, PackageEvent};
use std::collections::HashMap;
use std::time::Instant;

/// Check system health
///
/// # Errors
///
/// Returns an error if health check fails.
pub async fn check_health(ctx: &OpsCtx) -> Result<HealthCheck, Error> {
    let _start = Instant::now();

    ctx.emit(AppEvent::Package(PackageEvent::HealthCheckStarting));

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

    ctx.emit(AppEvent::Package(PackageEvent::HealthCheckCompleted {
        healthy: overall_healthy,
        issues: health_check
            .issues
            .iter()
            .map(|i| i.description.clone())
            .collect(),
    }));

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

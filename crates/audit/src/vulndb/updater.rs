//! Database update and synchronization logic

use sps2_errors::Error;
use sps2_events::{AppEvent, AuditEvent, EventEmitter, EventSender};
use sqlx::SqlitePool;

use super::sources::{update_from_github, update_from_nvd, update_from_osv};
use super::statistics::{get_vulnerability_count, update_last_update_time};

/// Update vulnerability database from all sources with event reporting
pub async fn update_database_from_sources(
    pool: &SqlitePool,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    let start_time = std::time::Instant::now();

    if let Some(sender) = &event_sender {
        sender.emit(AppEvent::Audit(AuditEvent::VulnDbUpdateStarted));
    }

    let mut sources_updated = 0;
    if let Err(e) = async {
        sources_updated += update_from_nvd_source(pool, event_sender).await?;
        sources_updated += update_from_osv_source(pool, event_sender).await?;
        sources_updated += update_from_github_source(pool, event_sender).await?;
        update_last_update_time(pool).await?;
        Result::<(), Error>::Ok(())
    }
    .await
    {
        if let Some(sender) = &event_sender {
            sender.emit(AppEvent::Audit(AuditEvent::VulnDbUpdateFailed {
                retryable: true,
            }));
        }
        return Err(e);
    }

    // Get final total from database
    let final_count = get_vulnerability_count(pool).await.unwrap_or(0);
    let total_duration = start_time.elapsed();

    if let Some(sender) = &event_sender {
        sender.emit(AppEvent::Audit(AuditEvent::VulnDbUpdateCompleted {
            total_vulnerabilities: final_count,
            sources_updated: sources_updated as usize,
            duration_ms: total_duration.as_millis().try_into().unwrap_or(u64::MAX),
        }));
    }

    Ok(())
}

/// Update from NVD source with error handling and events
async fn update_from_nvd_source(
    pool: &SqlitePool,
    _event_sender: Option<&EventSender>,
) -> Result<u32, Error> {
    match update_from_nvd(pool).await {
        Ok(_) => Ok(1),
        Err(e) => Err(e),
    }
}

/// Update from OSV source with error handling and events
async fn update_from_osv_source(
    pool: &SqlitePool,
    _event_sender: Option<&EventSender>,
) -> Result<u32, Error> {
    match update_from_osv(pool).await {
        Ok(_) => Ok(1),
        Err(e) => Err(e),
    }
}

/// Update from GitHub source with error handling and events
async fn update_from_github_source(
    pool: &SqlitePool,
    _event_sender: Option<&EventSender>,
) -> Result<u32, Error> {
    match update_from_github(pool).await {
        Ok(_) => Ok(1),
        Err(e) => Err(e),
    }
}

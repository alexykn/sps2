//! Database update and synchronization logic

use sps2_errors::Error;
use sps2_events::{Event, EventSender, EventSenderExt};
use sqlx::SqlitePool;

use super::sources::{update_from_github, update_from_nvd, update_from_osv};
use super::statistics::{get_vulnerability_count, update_last_update_time};

/// Update vulnerability database from all sources with event reporting
pub async fn update_database_from_sources(
    pool: &SqlitePool,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    let start_time = std::time::Instant::now();
    let mut sources_updated = 0;

    if let Some(sender) = &event_sender {
        sender.emit(Event::VulnDbUpdateStarting);
    }

    // Update from NVD
    sources_updated += update_from_nvd_source(pool, event_sender).await?;

    // Update from OSV
    sources_updated += update_from_osv_source(pool, event_sender).await?;

    // Update from GitHub Security Advisories
    sources_updated += update_from_github_source(pool, event_sender).await?;

    // Update metadata timestamp
    update_last_update_time(pool).await?;

    // Get final total from database
    let final_count = get_vulnerability_count(pool).await.unwrap_or(0);
    let total_duration = start_time.elapsed();

    if let Some(sender) = &event_sender {
        sender.emit(Event::VulnDbUpdateCompleted {
            total_vulnerabilities: final_count,
            sources_updated: sources_updated as usize,
            duration_ms: total_duration.as_millis() as u64,
        });
    }

    Ok(())
}

/// Update from NVD source with error handling and events
async fn update_from_nvd_source(
    pool: &SqlitePool,
    event_sender: Option<&EventSender>,
) -> Result<u32, Error> {
    if let Some(sender) = &event_sender {
        sender.emit(Event::VulnDbSourceUpdateStarting {
            source: "NVD".to_string(),
        });
    }

    let nvd_start = std::time::Instant::now();
    match update_from_nvd(pool).await {
        Ok(count) => {
            let duration = nvd_start.elapsed();

            if let Some(sender) = &event_sender {
                sender.emit(Event::VulnDbSourceUpdateCompleted {
                    source: "NVD".to_string(),
                    vulnerabilities_added: count,
                    duration_ms: duration.as_millis() as u64,
                });
            }
            Ok(1) // Return 1 to indicate source was successfully updated
        }
        Err(e) => {
            if let Some(sender) = &event_sender {
                sender.emit(Event::VulnDbSourceUpdateFailed {
                    source: "NVD".to_string(),
                    error: e.to_string(),
                });
            }
            Ok(0) // Return 0 to indicate source update failed
        }
    }
}

/// Update from OSV source with error handling and events
async fn update_from_osv_source(
    pool: &SqlitePool,
    event_sender: Option<&EventSender>,
) -> Result<u32, Error> {
    if let Some(sender) = &event_sender {
        sender.emit(Event::VulnDbSourceUpdateStarting {
            source: "OSV".to_string(),
        });
    }

    let osv_start = std::time::Instant::now();
    match update_from_osv(pool).await {
        Ok(count) => {
            let duration = osv_start.elapsed();

            if let Some(sender) = &event_sender {
                sender.emit(Event::VulnDbSourceUpdateCompleted {
                    source: "OSV".to_string(),
                    vulnerabilities_added: count,
                    duration_ms: duration.as_millis() as u64,
                });
            }
            Ok(1) // Return 1 to indicate source was successfully updated
        }
        Err(e) => {
            if let Some(sender) = &event_sender {
                sender.emit(Event::VulnDbSourceUpdateFailed {
                    source: "OSV".to_string(),
                    error: e.to_string(),
                });
            }
            Ok(0) // Return 0 to indicate source update failed
        }
    }
}

/// Update from GitHub source with error handling and events
async fn update_from_github_source(
    pool: &SqlitePool,
    event_sender: Option<&EventSender>,
) -> Result<u32, Error> {
    if let Some(sender) = &event_sender {
        sender.emit(Event::VulnDbSourceUpdateStarting {
            source: "GitHub".to_string(),
        });
    }

    let github_start = std::time::Instant::now();
    match update_from_github(pool).await {
        Ok(count) => {
            let duration = github_start.elapsed();

            if let Some(sender) = &event_sender {
                sender.emit(Event::VulnDbSourceUpdateCompleted {
                    source: "GitHub".to_string(),
                    vulnerabilities_added: count,
                    duration_ms: duration.as_millis() as u64,
                });
            }
            Ok(1) // Return 1 to indicate source was successfully updated
        }
        Err(e) => {
            if let Some(sender) = &event_sender {
                sender.emit(Event::VulnDbSourceUpdateFailed {
                    source: "GitHub".to_string(),
                    error: e.to_string(),
                });
            }
            Ok(0) // Return 0 to indicate source update failed
        }
    }
}

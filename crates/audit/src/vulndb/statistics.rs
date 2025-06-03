//! Database statistics and performance metrics

use sps2_errors::{AuditError, Error};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

/// Database statistics
#[derive(Debug, Clone)]
pub struct DatabaseStatistics {
    /// Number of vulnerabilities in database
    pub vulnerability_count: usize,
    /// Last update timestamp
    pub last_updated: Option<chrono::DateTime<chrono::Utc>>,
    /// Breakdown by severity
    pub severity_breakdown: HashMap<String, usize>,
}

/// Retrieve comprehensive database statistics
pub async fn get_statistics(pool: &SqlitePool) -> Result<DatabaseStatistics, Error> {
    let vulnerability_count = get_vulnerability_count(pool).await?;
    let last_updated = get_last_update_time(pool).await?;
    let severity_breakdown = get_severity_breakdown(pool).await?;

    Ok(DatabaseStatistics {
        vulnerability_count,
        last_updated,
        severity_breakdown,
    })
}

/// Get total vulnerability count from database
pub async fn get_vulnerability_count(pool: &SqlitePool) -> Result<usize, Error> {
    let count = sqlx::query("SELECT COUNT(*) as count FROM vulnerabilities")
        .fetch_one(pool)
        .await
        .map_err(|e| AuditError::DatabaseError {
            message: format!("Failed to get vulnerability count: {e}"),
        })?
        .get::<i64, _>("count") as usize;

    Ok(count)
}

/// Get last update time from metadata
pub async fn get_last_update_time(
    pool: &SqlitePool,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, Error> {
    let last_updated = sqlx::query("SELECT value FROM metadata WHERE key = 'last_update'")
        .fetch_optional(pool)
        .await?
        .and_then(|row| {
            row.get::<String, _>("value")
                .parse::<i64>()
                .ok()
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                .map(|dt| dt.with_timezone(&chrono::Utc))
        });

    Ok(last_updated)
}

/// Get severity breakdown counts
pub async fn get_severity_breakdown(pool: &SqlitePool) -> Result<HashMap<String, usize>, Error> {
    let severity_counts = sqlx::query(
        r"SELECT severity, COUNT(*) as count
          FROM vulnerabilities
          GROUP BY severity",
    )
    .fetch_all(pool)
    .await?;

    let mut severity_breakdown = HashMap::new();
    for row in severity_counts {
        let severity: String = row.get("severity");
        let count: i64 = row.get("count");
        severity_breakdown.insert(severity, count as usize);
    }

    Ok(severity_breakdown)
}

/// Check if database is fresh (updated recently)
pub async fn is_database_fresh(pool: &SqlitePool) -> Result<bool, Error> {
    // Check last update time from metadata
    let row = sqlx::query("SELECT value FROM metadata WHERE key = 'last_update'")
        .fetch_optional(pool)
        .await?;

    if let Some(row) = row {
        let last_update: i64 = row.get::<String, _>("value").parse().unwrap_or(0);
        let now = chrono::Utc::now().timestamp();
        let days_old = (now - last_update) / 86400; // seconds in a day

        // Consider fresh if updated within 7 days
        Ok(days_old < 7)
    } else {
        Ok(false)
    }
}

/// Update last update timestamp in metadata
pub async fn update_last_update_time(pool: &SqlitePool) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR REPLACE INTO metadata (key, value, updated_at) VALUES ('last_update', ?, ?)",
    )
    .bind(now.to_string())
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

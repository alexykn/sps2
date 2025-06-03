//! Database schema management and table creation

use sps2_errors::{AuditError, Error};
use sqlx::{Row, SqlitePool};

/// Create all database tables with proper schema
pub async fn create_tables(pool: &SqlitePool) -> Result<(), Error> {
    // Create metadata table
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(|e| AuditError::DatabaseError {
        message: format!("Failed to create metadata table: {e}"),
    })?;

    // Create vulnerabilities table
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS vulnerabilities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            cve_id TEXT UNIQUE NOT NULL,
            summary TEXT NOT NULL,
            severity TEXT NOT NULL,
            cvss_score REAL,
            published TEXT NOT NULL,
            modified TEXT NOT NULL,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(|e| AuditError::DatabaseError {
        message: format!("Failed to create vulnerabilities table: {e}"),
    })?;

    // Create affected packages table
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS affected_packages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            vulnerability_id INTEGER NOT NULL,
            package_name TEXT NOT NULL,
            package_type TEXT,
            affected_version TEXT,
            fixed_version TEXT,
            purl TEXT,
            cpe TEXT,
            FOREIGN KEY (vulnerability_id) REFERENCES vulnerabilities(id)
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(|e| AuditError::DatabaseError {
        message: format!("Failed to create affected_packages table: {e}"),
    })?;

    // Create references table
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS vulnerability_references (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            vulnerability_id INTEGER NOT NULL,
            url TEXT NOT NULL,
            reference_type TEXT,
            FOREIGN KEY (vulnerability_id) REFERENCES vulnerabilities(id)
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(|e| AuditError::DatabaseError {
        message: format!("Failed to create references table: {e}"),
    })?;

    // Create indexes for performance
    create_indexes(pool).await?;

    Ok(())
}

/// Create performance indexes
async fn create_indexes(pool: &SqlitePool) -> Result<(), Error> {
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_vulnerabilities_cve_id ON vulnerabilities(cve_id)")
        .execute(pool)
        .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_affected_packages_name ON affected_packages(package_name)",
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_affected_packages_purl ON affected_packages(purl)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_affected_packages_cpe ON affected_packages(cpe)")
        .execute(pool)
        .await?;

    Ok(())
}

/// Initialize metadata for new database
pub async fn initialize_metadata(pool: &SqlitePool) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();

    // Check if metadata already exists
    let existing = sqlx::query("SELECT COUNT(*) as count FROM metadata")
        .fetch_one(pool)
        .await
        .map_err(|e| AuditError::DatabaseError {
            message: format!("Failed to check metadata: {e}"),
        })?
        .get::<i64, _>("count");

    // Only initialize if no metadata exists
    if existing == 0 {
        // Set initial metadata
        sqlx::query("INSERT INTO metadata (key, value, updated_at) VALUES ('version', '1.0', ?)")
            .bind(now)
            .execute(pool)
            .await
            .map_err(|e| AuditError::DatabaseError {
                message: format!("Failed to set version metadata: {e}"),
            })?;

        sqlx::query("INSERT INTO metadata (key, value, updated_at) VALUES ('last_update', '0', ?)")
            .bind(now)
            .execute(pool)
            .await
            .map_err(|e| AuditError::DatabaseError {
                message: format!("Failed to set last_update metadata: {e}"),
            })?;
    }

    Ok(())
}

/// Configure database pragmas for optimal performance
pub async fn configure_pragmas(pool: &SqlitePool) -> Result<(), Error> {
    // Set pragmas for better performance
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(pool)
        .await?;
    sqlx::query("PRAGMA synchronous = NORMAL")
        .execute(pool)
        .await?;

    Ok(())
}

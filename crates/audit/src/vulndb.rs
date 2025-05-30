//! Vulnerability database management

use crate::types::Vulnerability;
use spsv2_errors::{AuditError, Error};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};

/// Vulnerability database manager
pub struct VulnDbManager {
    /// Database path
    db_path: PathBuf,
    /// Connection pool
    pool: Option<SqlitePool>,
}

impl VulnDbManager {
    /// Create new vulnerability database manager
    pub fn new(db_path: impl AsRef<Path>) -> Result<Self, Error> {
        let db_path = db_path.as_ref().to_path_buf();

        Ok(Self {
            db_path,
            pool: None,
        })
    }

    /// Initialize database connection
    pub async fn initialize(&mut self) -> Result<(), Error> {
        // Ensure database directory exists
        if let Some(parent) = self.db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let database_url = format!("sqlite:{}", self.db_path.display());

        // Create connection pool
        let pool =
            SqlitePool::connect(&database_url)
                .await
                .map_err(|e| AuditError::DatabaseError {
                    message: format!("Failed to connect to database: {e}"),
                })?;

        // Run migrations to create tables
        self.create_tables(&pool).await?;

        self.pool = Some(pool);
        Ok(())
    }

    /// Get the vulnerability database
    pub async fn get_database(&self) -> Result<VulnerabilityDatabase, Error> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| AuditError::DatabaseError {
                message: "Database not initialized".to_string(),
            })?;

        Ok(VulnerabilityDatabase::new(pool.clone()))
    }

    /// Update vulnerability database from sources
    pub async fn update(&mut self) -> Result<(), Error> {
        if self.pool.is_none() {
            self.initialize().await?;
        }

        // For now, this is a placeholder that would:
        // 1. Download CVE data from NVD, OSV, GitHub Security Advisories
        // 2. Parse and normalize the data
        // 3. Update the local SQLite database
        // 4. Handle incremental updates

        // Placeholder implementation
        Err(AuditError::NotImplemented {
            feature: "Vulnerability database updates".to_string(),
        }
        .into())
    }

    /// Check if database is fresh (updated recently)
    pub async fn is_fresh(&self) -> Result<bool, Error> {
        if self.pool.is_none() {
            return Ok(false);
        }

        // Check database metadata for last update time
        // For now, return false (needs implementation)
        Ok(false)
    }

    /// Create database tables
    async fn create_tables(&self, pool: &SqlitePool) -> Result<(), Error> {
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
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_vulnerabilities_cve_id ON vulnerabilities(cve_id)",
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_affected_packages_name ON affected_packages(package_name)")
            .execute(pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_affected_packages_purl ON affected_packages(purl)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_affected_packages_cpe ON affected_packages(cpe)",
        )
        .execute(pool)
        .await?;

        Ok(())
    }
}

/// Vulnerability database interface
pub struct VulnerabilityDatabase {
    /// Connection pool
    pool: SqlitePool,
}

impl VulnerabilityDatabase {
    /// Create new vulnerability database
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Find vulnerabilities by package name and version
    pub async fn find_vulnerabilities_by_package(
        &self,
        package_name: &str,
        package_version: &str,
    ) -> Result<Vec<Vulnerability>, Error> {
        // For now, return empty results since database is not populated
        // In the future, this would query the database for matching vulnerabilities
        let _ = (package_name, package_version);
        Ok(Vec::new())
    }

    /// Find vulnerabilities by PURL
    pub async fn find_vulnerabilities_by_purl(
        &self,
        purl: &str,
    ) -> Result<Vec<Vulnerability>, Error> {
        // Placeholder implementation
        let _ = purl;
        Ok(Vec::new())
    }

    /// Find vulnerabilities by CPE
    pub async fn find_vulnerabilities_by_cpe(
        &self,
        cpe: &str,
    ) -> Result<Vec<Vulnerability>, Error> {
        // Placeholder implementation
        let _ = cpe;
        Ok(Vec::new())
    }

    /// Get vulnerability by CVE ID
    pub async fn get_vulnerability_by_cve(
        &self,
        cve_id: &str,
    ) -> Result<Option<Vulnerability>, Error> {
        // Placeholder implementation
        let _ = cve_id;
        Ok(None)
    }

    /// Get database statistics
    pub async fn get_statistics(&self) -> Result<DatabaseStatistics, Error> {
        let vulnerability_count = sqlx::query("SELECT COUNT(*) as count FROM vulnerabilities")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AuditError::DatabaseError {
                message: format!("Failed to get vulnerability count: {e}"),
            })?
            .get::<i64, _>("count") as usize;

        Ok(DatabaseStatistics {
            vulnerability_count,
            last_updated: None, // Would be populated from metadata table
        })
    }
}

/// Database statistics
#[derive(Debug, Clone)]
pub struct DatabaseStatistics {
    /// Number of vulnerabilities in database
    pub vulnerability_count: usize,
    /// Last update timestamp
    pub last_updated: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_vulndb_manager_creation() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("vulndb.sqlite");

        let manager = VulnDbManager::new(&db_path);
        assert!(manager.is_ok());

        let manager = manager.unwrap();
        assert_eq!(manager.db_path, db_path);
        assert!(manager.pool.is_none());
    }

    #[tokio::test]
    async fn test_database_initialization() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.sqlite");

        let mut manager = VulnDbManager::new(&db_path).unwrap();

        // Initialize should succeed
        let result = manager.initialize().await;
        assert!(result.is_ok());
        assert!(manager.pool.is_some());

        // Database file should exist
        assert!(db_path.exists());
    }

    #[tokio::test]
    async fn test_database_freshness() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.sqlite");

        let mut manager = VulnDbManager::new(&db_path).unwrap();
        manager.initialize().await.unwrap();

        // Should not be fresh (no data)
        let fresh = manager.is_fresh().await.unwrap();
        assert!(!fresh);
    }

    #[tokio::test]
    async fn test_vulnerability_database() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.sqlite");

        let mut manager = VulnDbManager::new(&db_path).unwrap();
        manager.initialize().await.unwrap();

        let pool = manager.pool.unwrap();
        let db = VulnerabilityDatabase::new(pool);

        // Test statistics
        let stats = db.get_statistics().await.unwrap();
        assert_eq!(stats.vulnerability_count, 0);

        // Test empty queries
        let vulns = db
            .find_vulnerabilities_by_package("test", "1.0.0")
            .await
            .unwrap();
        assert!(vulns.is_empty());

        let vulns = db
            .find_vulnerabilities_by_purl("pkg:npm/test@1.0.0")
            .await
            .unwrap();
        assert!(vulns.is_empty());

        let vuln = db.get_vulnerability_by_cve("CVE-2023-1234").await.unwrap();
        assert!(vuln.is_none());
    }
}

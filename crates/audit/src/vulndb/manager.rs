//! Vulnerability database manager and core database operations

use crate::types::{Severity, Vulnerability};
use sps2_errors::{AuditError, Error};
use sps2_events::{Event, EventSender, EventSenderExt};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};

use super::sources::{update_from_github, update_from_nvd, update_from_osv};

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

    /// Get default vulnerability database path
    pub fn default_path() -> PathBuf {
        PathBuf::from("/opt/pm/vulndb/vulndb.sqlite")
    }

    /// Initialize database connection
    pub async fn initialize(&mut self) -> Result<(), Error> {
        // Ensure database directory exists
        if let Some(parent) = self.db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let database_url = format!("sqlite:{}?mode=rwc", self.db_path.display());

        // Create connection pool with options
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .map_err(|e| AuditError::DatabaseError {
                message: format!("Failed to connect to database: {e}"),
            })?;

        // Set pragmas for better performance
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA synchronous = NORMAL")
            .execute(&pool)
            .await?;

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

    /// Update vulnerability database from sources with event reporting
    ///
    /// # Panics
    ///
    /// Panics if the pool is `None` after initialization, which should never happen
    /// as `initialize()` sets up the pool or returns an error.
    pub async fn update(&mut self) -> Result<(), Error> {
        self.update_with_events(None).await
    }

    /// Update vulnerability database from sources with optional event reporting
    ///
    /// # Panics
    ///
    /// Panics if the pool is `None` after initialization, which should never happen
    /// as `initialize()` sets up the pool or returns an error.
    pub async fn update_with_events(
        &mut self,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        if self.pool.is_none() {
            self.initialize().await?;
        }

        let pool = self.pool.as_ref().expect("pool should be initialized");
        let start_time = std::time::Instant::now();
        let mut sources_updated = 0;

        if let Some(sender) = &event_sender {
            sender.emit(Event::VulnDbUpdateStarting);
        }

        // Update from NVD
        if let Some(sender) = &event_sender {
            sender.emit(Event::VulnDbSourceUpdateStarting {
                source: "NVD".to_string(),
            });
        }

        let nvd_start = std::time::Instant::now();
        match update_from_nvd(pool).await {
            Ok(count) => {
                sources_updated += 1;
                let duration = nvd_start.elapsed();

                if let Some(sender) = &event_sender {
                    sender.emit(Event::VulnDbSourceUpdateCompleted {
                        source: "NVD".to_string(),
                        vulnerabilities_added: count,
                        duration_ms: duration.as_millis() as u64,
                    });
                }
            }
            Err(e) => {
                if let Some(sender) = &event_sender {
                    sender.emit(Event::VulnDbSourceUpdateFailed {
                        source: "NVD".to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // Update from OSV
        if let Some(sender) = &event_sender {
            sender.emit(Event::VulnDbSourceUpdateStarting {
                source: "OSV".to_string(),
            });
        }

        let osv_start = std::time::Instant::now();
        match update_from_osv(pool).await {
            Ok(count) => {
                sources_updated += 1;
                let duration = osv_start.elapsed();

                if let Some(sender) = &event_sender {
                    sender.emit(Event::VulnDbSourceUpdateCompleted {
                        source: "OSV".to_string(),
                        vulnerabilities_added: count,
                        duration_ms: duration.as_millis() as u64,
                    });
                }
            }
            Err(e) => {
                if let Some(sender) = &event_sender {
                    sender.emit(Event::VulnDbSourceUpdateFailed {
                        source: "OSV".to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // Update from GitHub Security Advisories
        if let Some(sender) = &event_sender {
            sender.emit(Event::VulnDbSourceUpdateStarting {
                source: "GitHub".to_string(),
            });
        }

        let github_start = std::time::Instant::now();
        match update_from_github(pool).await {
            Ok(count) => {
                sources_updated += 1;
                let duration = github_start.elapsed();

                if let Some(sender) = &event_sender {
                    sender.emit(Event::VulnDbSourceUpdateCompleted {
                        source: "GitHub".to_string(),
                        vulnerabilities_added: count,
                        duration_ms: duration.as_millis() as u64,
                    });
                }
            }
            Err(e) => {
                if let Some(sender) = &event_sender {
                    sender.emit(Event::VulnDbSourceUpdateFailed {
                        source: "GitHub".to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // Update metadata
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR REPLACE INTO metadata (key, value, updated_at) VALUES ('last_update', ?, ?)",
        )
        .bind(now.to_string())
        .bind(now)
        .execute(pool)
        .await?;

        // Get final total from database
        let final_count = self.get_total_vulnerability_count().await.unwrap_or(0);
        let total_duration = start_time.elapsed();

        if let Some(sender) = &event_sender {
            sender.emit(Event::VulnDbUpdateCompleted {
                total_vulnerabilities: final_count,
                sources_updated,
                duration_ms: total_duration.as_millis() as u64,
            });
        }

        Ok(())
    }

    /// Get total vulnerability count from database
    async fn get_total_vulnerability_count(&self) -> Result<usize, Error> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| AuditError::DatabaseError {
                message: "Database not initialized".to_string(),
            })?;

        let count = sqlx::query("SELECT COUNT(*) as count FROM vulnerabilities")
            .fetch_one(pool)
            .await
            .map_err(|e| AuditError::DatabaseError {
                message: format!("Failed to get vulnerability count: {e}"),
            })?
            .get::<i64, _>("count") as usize;

        Ok(count)
    }

    /// Check if database is fresh (updated recently)
    pub async fn is_fresh(&self) -> Result<bool, Error> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| AuditError::DatabaseError {
                message: "Database not initialized".to_string(),
            })?;

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

    /// Create database tables
    async fn create_tables(&self, pool: &SqlitePool) -> Result<(), Error> {
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
        let rows = sqlx::query(
            r"
            SELECT DISTINCT v.cve_id, v.summary, v.severity, v.cvss_score,
                   v.published, v.modified, ap.affected_version, ap.fixed_version
            FROM vulnerabilities v
            INNER JOIN affected_packages ap ON v.id = ap.vulnerability_id
            WHERE ap.package_name = ? OR ap.package_name LIKE ?
            ",
        )
        .bind(package_name)
        .bind(format!("%/{package_name}")) // Match vendor/package patterns
        .fetch_all(&self.pool)
        .await?;

        let mut vulnerabilities = Vec::new();

        for row in rows {
            let affected_version: String = row.get("affected_version");
            let fixed_version: Option<String> = row.get("fixed_version");

            // Check if package version is affected
            if super::parser::is_version_affected(
                package_version,
                &affected_version,
                fixed_version.as_deref(),
            ) {
                let cve_id: String = row.get("cve_id");

                // Get references for this vulnerability
                let references = self.get_vulnerability_references(&cve_id).await?;

                let severity_str: String = row.get("severity");
                let severity = match severity_str.as_str() {
                    "critical" => Severity::Critical,
                    "high" => Severity::High,
                    "low" => Severity::Low,
                    _ => Severity::Medium,
                };

                vulnerabilities.push(Vulnerability {
                    cve_id,
                    summary: row.get("summary"),
                    severity,
                    cvss_score: row.get("cvss_score"),
                    affected_versions: vec![affected_version],
                    fixed_versions: fixed_version.into_iter().collect(),
                    published: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("published"),
                    )
                    .unwrap_or_else(|_| chrono::Utc::now().into())
                    .with_timezone(&chrono::Utc),
                    modified: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("modified"),
                    )
                    .unwrap_or_else(|_| chrono::Utc::now().into())
                    .with_timezone(&chrono::Utc),
                    references,
                });
            }
        }

        Ok(vulnerabilities)
    }

    /// Find vulnerabilities by PURL
    pub async fn find_vulnerabilities_by_purl(
        &self,
        purl: &str,
    ) -> Result<Vec<Vulnerability>, Error> {
        let rows = sqlx::query(
            r"
            SELECT DISTINCT v.cve_id, v.summary, v.severity, v.cvss_score,
                   v.published, v.modified, ap.affected_version, ap.fixed_version
            FROM vulnerabilities v
            INNER JOIN affected_packages ap ON v.id = ap.vulnerability_id
            WHERE ap.purl = ?
            ",
        )
        .bind(purl)
        .fetch_all(&self.pool)
        .await?;

        self.rows_to_vulnerabilities(rows).await
    }

    /// Find vulnerabilities by CPE
    pub async fn find_vulnerabilities_by_cpe(
        &self,
        cpe: &str,
    ) -> Result<Vec<Vulnerability>, Error> {
        let rows = sqlx::query(
            r"
            SELECT DISTINCT v.cve_id, v.summary, v.severity, v.cvss_score,
                   v.published, v.modified, ap.affected_version, ap.fixed_version
            FROM vulnerabilities v
            INNER JOIN affected_packages ap ON v.id = ap.vulnerability_id
            WHERE ap.cpe = ?
            ",
        )
        .bind(cpe)
        .fetch_all(&self.pool)
        .await?;

        self.rows_to_vulnerabilities(rows).await
    }

    /// Get vulnerability by CVE ID
    pub async fn get_vulnerability_by_cve(
        &self,
        cve_id: &str,
    ) -> Result<Option<Vulnerability>, Error> {
        let row = sqlx::query(
            r"
            SELECT cve_id, summary, severity, cvss_score, published, modified
            FROM vulnerabilities
            WHERE cve_id = ?
            ",
        )
        .bind(cve_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let references = self.get_vulnerability_references(cve_id).await?;
            let (affected_versions, fixed_versions) = self.get_affected_versions(cve_id).await?;

            let severity_str: String = row.get("severity");
            let severity = match severity_str.as_str() {
                "critical" => Severity::Critical,
                "high" => Severity::High,
                "low" => Severity::Low,
                _ => Severity::Medium,
            };

            Ok(Some(Vulnerability {
                cve_id: row.get("cve_id"),
                summary: row.get("summary"),
                severity,
                cvss_score: row.get("cvss_score"),
                affected_versions,
                fixed_versions,
                published: chrono::DateTime::parse_from_rfc3339(&row.get::<String, _>("published"))
                    .unwrap_or_else(|_| chrono::Utc::now().into())
                    .with_timezone(&chrono::Utc),
                modified: chrono::DateTime::parse_from_rfc3339(&row.get::<String, _>("modified"))
                    .unwrap_or_else(|_| chrono::Utc::now().into())
                    .with_timezone(&chrono::Utc),
                references,
            }))
        } else {
            Ok(None)
        }
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

        // Get last update time from metadata
        let last_updated = sqlx::query("SELECT value FROM metadata WHERE key = 'last_update'")
            .fetch_optional(&self.pool)
            .await?
            .and_then(|row| {
                row.get::<String, _>("value")
                    .parse::<i64>()
                    .ok()
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            });

        // Get severity breakdown
        let severity_counts = sqlx::query(
            r"SELECT severity, COUNT(*) as count
              FROM vulnerabilities
              GROUP BY severity",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut severity_breakdown = std::collections::HashMap::new();
        for row in severity_counts {
            let severity: String = row.get("severity");
            let count: i64 = row.get("count");
            severity_breakdown.insert(severity, count as usize);
        }

        Ok(DatabaseStatistics {
            vulnerability_count,
            last_updated,
            severity_breakdown,
        })
    }

    /// Get references for a vulnerability
    async fn get_vulnerability_references(&self, cve_id: &str) -> Result<Vec<String>, Error> {
        let rows = sqlx::query(
            r"
            SELECT url
            FROM vulnerability_references vr
            INNER JOIN vulnerabilities v ON vr.vulnerability_id = v.id
            WHERE v.cve_id = ?
            ",
        )
        .bind(cve_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|row| row.get("url")).collect())
    }

    /// Get affected and fixed versions for a vulnerability
    async fn get_affected_versions(
        &self,
        cve_id: &str,
    ) -> Result<(Vec<String>, Vec<String>), Error> {
        let rows = sqlx::query(
            r"
            SELECT affected_version, fixed_version
            FROM affected_packages ap
            INNER JOIN vulnerabilities v ON ap.vulnerability_id = v.id
            WHERE v.cve_id = ?
            ",
        )
        .bind(cve_id)
        .fetch_all(&self.pool)
        .await?;

        let mut affected = Vec::new();
        let mut fixed = Vec::new();

        for row in rows {
            let affected_version: String = row.get("affected_version");
            if !affected_version.is_empty() {
                affected.push(affected_version);
            }
            if let Ok(fixed_version) = row.try_get::<String, _>("fixed_version") {
                if !fixed_version.is_empty() {
                    fixed.push(fixed_version);
                }
            }
        }

        Ok((affected, fixed))
    }

    /// Convert database rows to vulnerabilities
    async fn rows_to_vulnerabilities(
        &self,
        rows: Vec<sqlx::sqlite::SqliteRow>,
    ) -> Result<Vec<Vulnerability>, Error> {
        use std::collections::HashMap;

        let mut vuln_map: HashMap<String, Vulnerability> = HashMap::new();

        for row in rows {
            let cve_id: String = row.get("cve_id");
            let affected_version: String = row.get("affected_version");
            let fixed_version: Option<String> = row.get("fixed_version");

            if let Some(vuln) = vuln_map.get_mut(&cve_id) {
                // Add versions to existing vulnerability
                if !affected_version.is_empty()
                    && !vuln.affected_versions.contains(&affected_version)
                {
                    vuln.affected_versions.push(affected_version);
                }
                if let Some(fv) = fixed_version {
                    if !fv.is_empty() && !vuln.fixed_versions.contains(&fv) {
                        vuln.fixed_versions.push(fv);
                    }
                }
            } else {
                // Create new vulnerability
                let references = self.get_vulnerability_references(&cve_id).await?;

                let severity_str: String = row.get("severity");
                let severity = match severity_str.as_str() {
                    "critical" => Severity::Critical,
                    "high" => Severity::High,
                    "low" => Severity::Low,
                    _ => Severity::Medium,
                };

                let vuln = Vulnerability {
                    cve_id: cve_id.clone(),
                    summary: row.get("summary"),
                    severity,
                    cvss_score: row.get("cvss_score"),
                    affected_versions: if affected_version.is_empty() {
                        vec![]
                    } else {
                        vec![affected_version]
                    },
                    fixed_versions: fixed_version
                        .into_iter()
                        .filter(|v| !v.is_empty())
                        .collect(),
                    published: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("published"),
                    )
                    .unwrap_or_else(|_| chrono::Utc::now().into())
                    .with_timezone(&chrono::Utc),
                    modified: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<String, _>("modified"),
                    )
                    .unwrap_or_else(|_| chrono::Utc::now().into())
                    .with_timezone(&chrono::Utc),
                    references,
                };

                vuln_map.insert(cve_id, vuln);
            }
        }

        Ok(vuln_map.into_values().collect())
    }
}

/// Database statistics
#[derive(Debug, Clone)]
pub struct DatabaseStatistics {
    /// Number of vulnerabilities in database
    pub vulnerability_count: usize,
    /// Last update timestamp
    pub last_updated: Option<chrono::DateTime<chrono::Utc>>,
    /// Breakdown by severity
    pub severity_breakdown: std::collections::HashMap<String, usize>,
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

        let db = manager.get_database().await.unwrap();

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

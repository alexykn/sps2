//! Core vulnerability database operations and queries

use crate::types::{Severity, Vulnerability};
use sps2_errors::Error;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

use super::statistics::{get_statistics, DatabaseStatistics};

/// Vulnerability database interface for queries and operations
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
                let severity = parse_severity(&severity_str);

                vulnerabilities.push(Vulnerability {
                    cve_id,
                    summary: row.get("summary"),
                    severity,
                    cvss_score: row.get("cvss_score"),
                    affected_versions: vec![affected_version],
                    fixed_versions: fixed_version.into_iter().collect(),
                    published: parse_datetime(&row.get::<String, _>("published")),
                    modified: parse_datetime(&row.get::<String, _>("modified")),
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
            let severity = parse_severity(&severity_str);

            Ok(Some(Vulnerability {
                cve_id: row.get("cve_id"),
                summary: row.get("summary"),
                severity,
                cvss_score: row.get("cvss_score"),
                affected_versions,
                fixed_versions,
                published: parse_datetime(&row.get::<String, _>("published")),
                modified: parse_datetime(&row.get::<String, _>("modified")),
                references,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get database statistics
    pub async fn get_statistics(&self) -> Result<DatabaseStatistics, Error> {
        get_statistics(&self.pool).await
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
                let severity = parse_severity(&severity_str);

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
                    published: parse_datetime(&row.get::<String, _>("published")),
                    modified: parse_datetime(&row.get::<String, _>("modified")),
                    references,
                };

                vuln_map.insert(cve_id, vuln);
            }
        }

        Ok(vuln_map.into_values().collect())
    }
}

/// Parse severity string to Severity enum
fn parse_severity(severity_str: &str) -> Severity {
    match severity_str {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "low" => Severity::Low,
        _ => Severity::Medium,
    }
}

/// Parse datetime string to chrono `DateTime`
fn parse_datetime(datetime_str: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(datetime_str)
        .unwrap_or_else(|_| chrono::Utc::now().into())
        .with_timezone(&chrono::Utc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_vulnerability_database() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.sqlite");

        // Initialize a basic database
        let database_url = format!("sqlite:{}?mode=rwc", db_path.display());
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .unwrap();

        // Create tables
        super::super::schema::create_tables(&pool).await.unwrap();

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

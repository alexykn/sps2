//! Vulnerability database management

use crate::types::{Severity, Vulnerability};
use spsv2_errors::{AuditError, Error};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

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

    /// Update vulnerability database from sources
    pub async fn update(&mut self) -> Result<(), Error> {
        if self.pool.is_none() {
            self.initialize().await?;
        }

        let pool = self.pool.as_ref().unwrap();
        let mut update_count = 0;

        // Update from NVD
        match self.update_from_nvd(pool).await {
            Ok(count) => {
                update_count += count;
                eprintln!("Updated {} vulnerabilities from NVD", count);
            }
            Err(e) => eprintln!("Failed to update from NVD: {}", e),
        }

        // Update from OSV
        match self.update_from_osv(pool).await {
            Ok(count) => {
                update_count += count;
                eprintln!("Updated {} vulnerabilities from OSV", count);
            }
            Err(e) => eprintln!("Failed to update from OSV: {}", e),
        }

        // Update from GitHub Security Advisories
        match self.update_from_github(pool).await {
            Ok(count) => {
                update_count += count;
                eprintln!("Updated {} vulnerabilities from GitHub", count);
            }
            Err(e) => eprintln!("Failed to update from GitHub: {}", e),
        }

        // Update metadata
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT OR REPLACE INTO metadata (key, value, updated_at) VALUES ('last_update', ?, ?)")
            .bind(now.to_string())
            .bind(now)
            .execute(pool)
            .await?;

        eprintln!("Total vulnerabilities updated: {}", update_count);
        Ok(())
    }

    /// Check if database is fresh (updated recently)
    pub async fn is_fresh(&self) -> Result<bool, Error> {
        let pool = self.pool.as_ref().ok_or_else(|| AuditError::DatabaseError {
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

    /// Update from NVD API
    async fn update_from_nvd(&self, pool: &SqlitePool) -> Result<usize, Error> {
        let client = reqwest::Client::new();
        let mut total_updated = 0;
        let mut start_index = 0;
        let results_per_page = 2000;

        loop {
            // NVD API endpoint - fetch recent CVEs
            let url = format!(
                "https://services.nvd.nist.gov/rest/json/cves/2.0?resultsPerPage={}&startIndex={}",
                results_per_page, start_index
            );

            let response = client
                .get(&url)
                .header("User-Agent", "spsv2-package-manager")
                .send()
                .await
                .map_err(|e| AuditError::CveFetchError {
                    message: format!("Failed to fetch from NVD: {e}"),
                })?;

            if !response.status().is_success() {
                return Err(AuditError::CveFetchError {
                    message: format!("NVD API returned status: {}", response.status()),
                }
                .into());
            }

            let data: serde_json::Value = response.json().await?;
            
            let vulnerabilities = data["vulnerabilities"].as_array().unwrap_or(&Vec::new());
            if vulnerabilities.is_empty() {
                break;
            }

            for vuln in vulnerabilities {
                if let Some(cve) = vuln["cve"].as_object() {
                    let count = self.insert_nvd_vulnerability(pool, cve).await?;
                    total_updated += count;
                }
            }

            start_index += results_per_page;
            
            // Check if we've fetched all results
            let total_results = data["totalResults"].as_u64().unwrap_or(0);
            if start_index >= total_results as usize {
                break;
            }

            // Be respectful to the API
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        Ok(total_updated)
    }

    /// Insert NVD vulnerability into database
    async fn insert_nvd_vulnerability(
        &self,
        pool: &SqlitePool,
        cve: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<usize, Error> {
        let cve_id = cve["id"].as_str().unwrap_or("");
        if cve_id.is_empty() {
            return Ok(0);
        }

        // Extract basic information
        let descriptions = cve["descriptions"]
            .as_array()
            .and_then(|arr| arr.iter().find(|d| d["lang"].as_str() == Some("en")))
            .and_then(|d| d["value"].as_str())
            .unwrap_or("No description available");

        // Extract metrics
        let (severity, cvss_score) = extract_nvd_severity(cve);

        let published = cve["published"]
            .as_str()
            .unwrap_or("1970-01-01T00:00:00.000Z");
        let modified = cve["lastModified"]
            .as_str()
            .unwrap_or(published);

        // Insert vulnerability
        let result = sqlx::query(
            "INSERT OR REPLACE INTO vulnerabilities (cve_id, summary, severity, cvss_score, published, modified) 
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(cve_id)
        .bind(descriptions)
        .bind(severity)
        .bind(cvss_score)
        .bind(published)
        .bind(modified)
        .execute(pool)
        .await?;

        let vuln_id = result.last_insert_rowid();

        // Insert affected configurations
        if let Some(configurations) = cve["configurations"].as_array() {
            for config in configurations {
                if let Some(nodes) = config["nodes"].as_array() {
                    self.process_nvd_nodes(pool, vuln_id, nodes).await?;
                }
            }
        }

        // Insert references
        if let Some(references) = cve["references"].as_array() {
            for reference in references {
                if let Some(url) = reference["url"].as_str() {
                    sqlx::query(
                        "INSERT OR IGNORE INTO vulnerability_references (vulnerability_id, url, reference_type) 
                         VALUES (?, ?, ?)",
                    )
                    .bind(vuln_id)
                    .bind(url)
                    .bind("nvd")
                    .execute(pool)
                    .await?;
                }
            }
        }

        Ok(1)
    }

    /// Process NVD configuration nodes
    async fn process_nvd_nodes(
        &self,
        pool: &SqlitePool,
        vuln_id: i64,
        nodes: &[serde_json::Value],
    ) -> Result<(), Error> {
        for node in nodes {
            if let Some(cpe_matches) = node["cpeMatch"].as_array() {
                for cpe_match in cpe_matches {
                    if cpe_match["vulnerable"].as_bool() != Some(true) {
                        continue;
                    }

                    let cpe = cpe_match["criteria"].as_str().unwrap_or("");
                    if cpe.is_empty() {
                        continue;
                    }

                    // Parse CPE to extract package information
                    let parts: Vec<&str> = cpe.split(':').collect();
                    if parts.len() >= 5 {
                        let package_type = parts[2];
                        let vendor = parts[3];
                        let product = parts[4];
                        let version = parts.get(5).unwrap_or(&"*");

                        let package_name = if vendor == "*" || vendor == product {
                            product.to_string()
                        } else {
                            format!("{}/{}", vendor, product)
                        };

                        sqlx::query(
                            "INSERT OR IGNORE INTO affected_packages 
                             (vulnerability_id, package_name, package_type, affected_version, cpe) 
                             VALUES (?, ?, ?, ?, ?)",
                        )
                        .bind(vuln_id)
                        .bind(&package_name)
                        .bind(package_type)
                        .bind(version)
                        .bind(cpe)
                        .execute(pool)
                        .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Update from OSV database
    async fn update_from_osv(&self, pool: &SqlitePool) -> Result<usize, Error> {
        let client = reqwest::Client::new();
        let temp_dir = tempfile::tempdir()?;
        let zip_path = temp_dir.path().join("osv-all.zip");

        // Download OSV database
        let response = client
            .get("https://osv-vulnerabilities.storage.googleapis.com/all.zip")
            .header("User-Agent", "spsv2-package-manager")
            .send()
            .await
            .map_err(|e| AuditError::CveFetchError {
                message: format!("Failed to download OSV database: {e}"),
            })?;

        if !response.status().is_success() {
            return Err(AuditError::CveFetchError {
                message: format!("OSV download returned status: {}", response.status()),
            }
            .into());
        }

        // Save to file
        let bytes = response.bytes().await?;
        let mut file = tokio::fs::File::create(&zip_path).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;
        drop(file);

        // Extract and process
        let zip_file = std::fs::File::open(&zip_path)?;
        let mut archive = zip::ZipArchive::new(zip_file)?;
        let mut total_updated = 0;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            if !file.name().ends_with(".json") {
                continue;
            }

            let mut contents = String::new();
            std::io::Read::read_to_string(&mut file, &mut contents)?;

            if let Ok(vuln) = serde_json::from_str::<serde_json::Value>(&contents) {
                let count = self.insert_osv_vulnerability(pool, &vuln).await?;
                total_updated += count;
            }
        }

        Ok(total_updated)
    }

    /// Insert OSV vulnerability into database
    async fn insert_osv_vulnerability(
        &self,
        pool: &SqlitePool,
        vuln: &serde_json::Value,
    ) -> Result<usize, Error> {
        let id = vuln["id"].as_str().unwrap_or("");
        if id.is_empty() {
            return Ok(0);
        }

        let summary = vuln["summary"]
            .as_str()
            .or_else(|| vuln["details"].as_str())
            .unwrap_or("No description available");

        let severity = vuln["database_specific"]["severity"]
            .as_str()
            .unwrap_or("medium")
            .to_lowercase();

        let published = vuln["published"]
            .as_str()
            .unwrap_or("1970-01-01T00:00:00Z");
        let modified = vuln["modified"].as_str().unwrap_or(published);

        // Insert vulnerability
        let result = sqlx::query(
            "INSERT OR REPLACE INTO vulnerabilities (cve_id, summary, severity, published, modified) 
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(summary)
        .bind(&severity)
        .bind(published)
        .bind(modified)
        .execute(pool)
        .await?;

        let vuln_id = result.last_insert_rowid();

        // Process affected packages
        if let Some(affected) = vuln["affected"].as_array() {
            for pkg in affected {
                let package_name = pkg["package"]["name"].as_str().unwrap_or("");
                let ecosystem = pkg["package"]["ecosystem"].as_str().unwrap_or("");
                let purl = pkg["package"]["purl"].as_str();

                if let Some(ranges) = pkg["ranges"].as_array() {
                    for range in ranges {
                        if let Some(events) = range["events"].as_array() {
                            let mut affected_version = String::new();
                            let mut fixed_version = String::new();

                            for event in events {
                                if let Some(introduced) = event["introduced"].as_str() {
                                    affected_version = format!(">={}", introduced);
                                }
                                if let Some(fixed) = event["fixed"].as_str() {
                                    fixed_version = fixed.to_string();
                                }
                            }

                            sqlx::query(
                                "INSERT OR IGNORE INTO affected_packages 
                                 (vulnerability_id, package_name, package_type, affected_version, fixed_version, purl) 
                                 VALUES (?, ?, ?, ?, ?, ?)",
                            )
                            .bind(vuln_id)
                            .bind(package_name)
                            .bind(ecosystem)
                            .bind(&affected_version)
                            .bind(&fixed_version)
                            .bind(purl)
                            .execute(pool)
                            .await?;
                        }
                    }
                }
            }
        }

        // Insert references
        if let Some(references) = vuln["references"].as_array() {
            for reference in references {
                if let Some(url) = reference["url"].as_str() {
                    let ref_type = reference["type"].as_str().unwrap_or("osv");
                    sqlx::query(
                        "INSERT OR IGNORE INTO vulnerability_references (vulnerability_id, url, reference_type) 
                         VALUES (?, ?, ?)",
                    )
                    .bind(vuln_id)
                    .bind(url)
                    .bind(ref_type)
                    .execute(pool)
                    .await?;
                }
            }
        }

        Ok(1)
    }

    /// Update from GitHub Security Advisories
    async fn update_from_github(&self, pool: &SqlitePool) -> Result<usize, Error> {
        let client = reqwest::Client::new();
        let mut total_updated = 0;
        let mut has_next_page = true;
        let mut end_cursor: Option<String> = None;

        // GitHub GraphQL API endpoint
        let url = "https://api.github.com/graphql";

        while has_next_page {
            let query = build_github_query(end_cursor.as_deref());

            let response = client
                .post(url)
                .header("User-Agent", "spsv2-package-manager")
                .header("Accept", "application/vnd.github.v4+json")
                .json(&serde_json::json!({ "query": query }))
                .send()
                .await
                .map_err(|e| AuditError::CveFetchError {
                    message: format!("Failed to fetch from GitHub: {e}"),
                })?;

            if !response.status().is_success() {
                return Err(AuditError::CveFetchError {
                    message: format!("GitHub API returned status: {}", response.status()),
                }
                .into());
            }

            let data: serde_json::Value = response.json().await?;

            if let Some(advisories) = data["data"]["securityAdvisories"]["nodes"].as_array() {
                for advisory in advisories {
                    let count = self.insert_github_advisory(pool, advisory).await?;
                    total_updated += count;
                }
            }

            // Check pagination
            has_next_page = data["data"]["securityAdvisories"]["pageInfo"]["hasNextPage"]
                .as_bool()
                .unwrap_or(false);
            end_cursor = data["data"]["securityAdvisories"]["pageInfo"]["endCursor"]
                .as_str()
                .map(String::from);

            if has_next_page {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        Ok(total_updated)
    }

    /// Insert GitHub advisory into database
    async fn insert_github_advisory(
        &self,
        pool: &SqlitePool,
        advisory: &serde_json::Value,
    ) -> Result<usize, Error> {
        let ghsa_id = advisory["ghsaId"].as_str().unwrap_or("");
        if ghsa_id.is_empty() {
            return Ok(0);
        }

        let summary = advisory["summary"].as_str().unwrap_or("No summary");
        let severity = advisory["severity"]
            .as_str()
            .unwrap_or("medium")
            .to_lowercase();

        let published = advisory["publishedAt"]
            .as_str()
            .unwrap_or("1970-01-01T00:00:00Z");
        let updated = advisory["updatedAt"].as_str().unwrap_or(published);

        // Extract CVSS score if available
        let cvss_score = advisory["cvss"]["score"].as_f64().map(|s| s as f32);

        // Insert vulnerability
        let result = sqlx::query(
            "INSERT OR REPLACE INTO vulnerabilities (cve_id, summary, severity, cvss_score, published, modified) 
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(ghsa_id)
        .bind(summary)
        .bind(&severity)
        .bind(cvss_score)
        .bind(published)
        .bind(updated)
        .execute(pool)
        .await?;

        let vuln_id = result.last_insert_rowid();

        // Process vulnerabilities
        if let Some(vulnerabilities) = advisory["vulnerabilities"]["nodes"].as_array() {
            for vuln in vulnerabilities {
                let package_name = vuln["package"]["name"].as_str().unwrap_or("");
                let ecosystem = vuln["package"]["ecosystem"].as_str().unwrap_or("");

                let affected_range = vuln["vulnerableVersionRange"].as_str().unwrap_or("");
                let first_patched = vuln["firstPatchedVersion"]["identifier"]
                    .as_str()
                    .unwrap_or("");

                sqlx::query(
                    "INSERT OR IGNORE INTO affected_packages 
                     (vulnerability_id, package_name, package_type, affected_version, fixed_version) 
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(vuln_id)
                .bind(package_name)
                .bind(ecosystem)
                .bind(affected_range)
                .bind(first_patched)
                .execute(pool)
                .await?;
            }
        }

        // Insert references
        if let Some(references) = advisory["references"]["nodes"].as_array() {
            for reference in references {
                if let Some(url) = reference["url"].as_str() {
                    sqlx::query(
                        "INSERT OR IGNORE INTO vulnerability_references (vulnerability_id, url, reference_type) 
                         VALUES (?, ?, ?)",
                    )
                    .bind(vuln_id)
                    .bind(url)
                    .bind("github")
                    .execute(pool)
                    .await?;
                }
            }
        }

        Ok(1)
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
        .bind(format!("%/{}", package_name)) // Match vendor/package patterns
        .fetch_all(&self.pool)
        .await?;

        let mut vulnerabilities = Vec::new();
        
        for row in rows {
            let affected_version: String = row.get("affected_version");
            let fixed_version: Option<String> = row.get("fixed_version");
            
            // Check if package version is affected
            if is_version_affected(package_version, &affected_version, fixed_version.as_deref()) {
                let cve_id: String = row.get("cve_id");
                
                // Get references for this vulnerability
                let references = self.get_vulnerability_references(&cve_id).await?;
                
                let severity_str: String = row.get("severity");
                let severity = match severity_str.as_str() {
                    "critical" => Severity::Critical,
                    "high" => Severity::High,
                    "medium" => Severity::Medium,
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
                    published: chrono::DateTime::parse_from_rfc3339(&row.get::<String, _>("published"))
                        .unwrap_or_else(|_| chrono::Utc::now().into())
                        .with_timezone(&chrono::Utc),
                    modified: chrono::DateTime::parse_from_rfc3339(&row.get::<String, _>("modified"))
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
                "medium" => Severity::Medium,
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

        Ok(DatabaseStatistics {
            vulnerability_count,
            last_updated: None, // Would be populated from metadata table
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
    async fn get_affected_versions(&self, cve_id: &str) -> Result<(Vec<String>, Vec<String>), Error> {
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
    async fn rows_to_vulnerabilities(&self, rows: Vec<sqlx::sqlite::SqliteRow>) -> Result<Vec<Vulnerability>, Error> {
        use std::collections::HashMap;
        
        let mut vuln_map: HashMap<String, Vulnerability> = HashMap::new();
        
        for row in rows {
            let cve_id: String = row.get("cve_id");
            let affected_version: String = row.get("affected_version");
            let fixed_version: Option<String> = row.get("fixed_version");
            
            if let Some(vuln) = vuln_map.get_mut(&cve_id) {
                // Add versions to existing vulnerability
                if !affected_version.is_empty() && !vuln.affected_versions.contains(&affected_version) {
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
                    "medium" => Severity::Medium,
                    "low" => Severity::Low,
                    _ => Severity::Medium,
                };
                
                let vuln = Vulnerability {
                    cve_id: cve_id.clone(),
                    summary: row.get("summary"),
                    severity,
                    cvss_score: row.get("cvss_score"),
                    affected_versions: if affected_version.is_empty() { vec![] } else { vec![affected_version] },
                    fixed_versions: fixed_version.into_iter().filter(|v| !v.is_empty()).collect(),
                    published: chrono::DateTime::parse_from_rfc3339(&row.get::<String, _>("published"))
                        .unwrap_or_else(|_| chrono::Utc::now().into())
                        .with_timezone(&chrono::Utc),
                    modified: chrono::DateTime::parse_from_rfc3339(&row.get::<String, _>("modified"))
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
}

/// Build GitHub GraphQL query for security advisories
fn build_github_query(cursor: Option<&str>) -> String {
    let after = cursor
        .map(|c| format!(r#", after: "{}""#, c))
        .unwrap_or_default();

    format!(
        r#"
        query {{
            securityAdvisories(first: 100{}) {{
                pageInfo {{
                    hasNextPage
                    endCursor
                }}
                nodes {{
                    ghsaId
                    summary
                    severity
                    publishedAt
                    updatedAt
                    cvss {{
                        score
                    }}
                    vulnerabilities(first: 10) {{
                        nodes {{
                            package {{
                                name
                                ecosystem
                            }}
                            vulnerableVersionRange
                            firstPatchedVersion {{
                                identifier
                            }}
                        }}
                    }}
                    references(first: 10) {{
                        nodes {{
                            url
                        }}
                    }}
                }}
            }}
        }}
        "#,
        after
    )
}

/// Check if a version is affected by vulnerability
fn is_version_affected(version: &str, affected_range: &str, fixed_version: Option<&str>) -> bool {
    // Simple version checking - in production, this would use proper version parsing
    // and range checking with semver
    
    if affected_range == "*" || affected_range.is_empty() {
        // All versions affected unless there's a fix
        if let Some(fixed) = fixed_version {
            // Compare versions - simplified for now
            version < fixed
        } else {
            true
        }
    } else if affected_range.starts_with(">=") {
        let min_version = affected_range.trim_start_matches(">=").trim();
        if let Some(fixed) = fixed_version {
            version >= min_version && version < fixed
        } else {
            version >= min_version
        }
    } else if affected_range.starts_with('<') {
        let max_version = affected_range.trim_start_matches('<').trim();
        version < max_version
    } else if affected_range.starts_with('=') {
        let exact_version = affected_range.trim_start_matches('=').trim();
        version == exact_version
    } else {
        // For complex ranges, default to affected for safety
        true
    }
}

/// Extract severity and CVSS score from NVD data
fn extract_nvd_severity(cve: &serde_json::Map<String, serde_json::Value>) -> (&'static str, Option<f32>) {
    // Try CVSS v3 first
    if let Some(metrics) = cve["metrics"]["cvssMetricV31"].as_array() {
        if let Some(metric) = metrics.first() {
            let severity = metric["cvssData"]["baseSeverity"]
                .as_str()
                .unwrap_or("medium")
                .to_lowercase();
            let score = metric["cvssData"]["baseScore"].as_f64().map(|s| s as f32);
            
            let severity_str = match severity.as_str() {
                "critical" => "critical",
                "high" => "high",
                "medium" => "medium",
                "low" => "low",
                _ => "medium",
            };
            
            return (severity_str, score);
        }
    }

    // Fall back to CVSS v2
    if let Some(metrics) = cve["metrics"]["cvssMetricV2"].as_array() {
        if let Some(metric) = metrics.first() {
            let score = metric["cvssData"]["baseScore"].as_f64().map(|s| s as f32);
            let severity = match score {
                Some(s) if s >= 9.0 => "critical",
                Some(s) if s >= 7.0 => "high",
                Some(s) if s >= 4.0 => "medium",
                Some(_) => "low",
                None => "medium",
            };
            return (severity, score);
        }
    }

    ("medium", None)
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

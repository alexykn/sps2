//! NVD (National Vulnerability Database) source implementation

use sps2_errors::{AuditError, Error};
use sqlx::SqlitePool;

use crate::vulndb::parser::extract_nvd_severity;

/// Update from NVD API
pub(crate) async fn update_from_nvd(pool: &SqlitePool) -> Result<usize, Error> {
    let client = reqwest::Client::new();
    let mut total_updated = 0;
    let mut start_index = 0;
    let results_per_page = 2000;

    loop {
        // NVD API endpoint - fetch recent CVEs
        let url = format!(
            "https://services.nvd.nist.gov/rest/json/cves/2.0?resultsPerPage={results_per_page}&startIndex={start_index}"
        );

        let response = client
            .get(&url)
            .header("User-Agent", "sps2-package-manager")
            .send()
            .await
            .map_err(|e| AuditError::CveFetchError {
                message: format!("Failed to fetch from NVD: {e}"),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AuditError::CveFetchError {
                message: format!("NVD API returned status: {status}"),
            }
            .into());
        }

        let data: serde_json::Value =
            response
                .json()
                .await
                .map_err(|e| AuditError::CveFetchError {
                    message: format!("Failed to parse NVD JSON: {e}"),
                })?;

        let Some(vulnerabilities) = data.get("vulnerabilities").and_then(|v| v.as_array()) else {
            break;
        };
        if vulnerabilities.is_empty() {
            break;
        }

        for vuln in vulnerabilities {
            if let Some(cve) = vuln.get("cve").and_then(|v| v.as_object()) {
                let count = insert_nvd_vulnerability(pool, cve).await?;
                total_updated += count;
            }
        }

        start_index += results_per_page;

        // Check if we've fetched all results
        let total_results = data
            .get("totalResults")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
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
    pool: &SqlitePool,
    cve: &serde_json::Map<String, serde_json::Value>,
) -> Result<usize, Error> {
    let cve_id = cve.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if cve_id.is_empty() {
        return Ok(0);
    }

    // Extract basic information
    let descriptions = cve
        .get("descriptions")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|d| d.get("lang").and_then(|v| v.as_str()) == Some("en"))
        })
        .and_then(|d| d.get("value").and_then(|v| v.as_str()))
        .unwrap_or("No description available");

    // Extract metrics
    let (severity, cvss_score) = extract_nvd_severity(cve);

    let published = cve
        .get("published")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00.000Z");
    let modified = cve
        .get("lastModified")
        .and_then(|v| v.as_str())
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
    if let Some(configurations) = cve.get("configurations").and_then(|v| v.as_array()) {
        for config in configurations {
            if let Some(nodes) = config.get("nodes").and_then(|v| v.as_array()) {
                process_nvd_nodes(pool, vuln_id, nodes).await?;
            }
        }
    }

    // Insert references
    if let Some(references) = cve.get("references").and_then(|v| v.as_array()) {
        for reference in references {
            if let Some(url) = reference.get("url").and_then(|v| v.as_str()) {
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
    pool: &SqlitePool,
    vuln_id: i64,
    nodes: &[serde_json::Value],
) -> Result<(), Error> {
    for node in nodes {
        if let Some(cpe_matches) = node.get("cpeMatch").and_then(|v| v.as_array()) {
            for cpe_match in cpe_matches {
                if cpe_match
                    .get("vulnerable")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
                {
                    continue;
                }

                let cpe = cpe_match
                    .get("criteria")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
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
                        format!("{vendor}/{product}")
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

//! OSV (Open Source Vulnerabilities) database source implementation

use sps2_errors::{AuditError, Error};
use sqlx::SqlitePool;
use std::io::Read;
use tokio::io::AsyncWriteExt;

/// Update from OSV database
pub(crate) async fn update_from_osv(pool: &SqlitePool) -> Result<usize, Error> {
    let client = reqwest::Client::new();
    let temp_dir = tempfile::tempdir()?;
    let zip_path = temp_dir.path().join("osv-all.zip");

    // Download OSV database
    let response = client
        .get("https://osv-vulnerabilities.storage.googleapis.com/all.zip")
        .header("User-Agent", "sps2-package-manager")
        .send()
        .await
        .map_err(|e| AuditError::CveFetchError {
            message: format!("Failed to download OSV database: {e}"),
        })?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(AuditError::CveFetchError {
            message: format!("OSV download returned status: {status}"),
        }
        .into());
    }

    // Save to file
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AuditError::CveFetchError {
            message: format!("Failed to download OSV bytes: {e}"),
        })?;
    let mut file = tokio::fs::File::create(&zip_path).await?;
    file.write_all(&bytes).await?;
    file.flush().await?;
    drop(file);

    // Extract and process
    let zip_file = std::fs::File::open(&zip_path)?;
    let mut archive = zip::ZipArchive::new(zip_file).map_err(|e| AuditError::CveFetchError {
        message: format!("Failed to open zip archive: {e}"),
    })?;
    let mut total_updated = 0;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| AuditError::CveFetchError {
            message: format!("Failed to extract file from archive: {e}"),
        })?;
        if !std::path::Path::new(file.name())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            continue;
        }

        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        if let Ok(vuln) = serde_json::from_str::<serde_json::Value>(&contents) {
            let count = insert_osv_vulnerability(pool, &vuln).await?;
            total_updated += count;
        }
    }

    Ok(total_updated)
}

/// Insert OSV vulnerability into database
async fn insert_osv_vulnerability(
    pool: &SqlitePool,
    vuln: &serde_json::Value,
) -> Result<usize, Error> {
    let id = vuln.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() {
        return Ok(0);
    }

    let summary = vuln
        .get("summary")
        .and_then(|v| v.as_str())
        .or_else(|| vuln.get("details").and_then(|v| v.as_str()))
        .unwrap_or("No description available");

    let severity = vuln
        .get("database_specific")
        .and_then(|db| db.get("severity"))
        .and_then(|v| v.as_str())
        .unwrap_or("medium")
        .to_lowercase();

    let published = vuln
        .get("published")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");
    let modified = vuln
        .get("modified")
        .and_then(|v| v.as_str())
        .unwrap_or(published);

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
    if let Some(affected) = vuln.get("affected").and_then(|v| v.as_array()) {
        for pkg in affected {
            process_osv_package(pool, vuln_id, pkg).await?;
        }
    }

    // Insert references
    if let Some(references) = vuln.get("references").and_then(|v| v.as_array()) {
        for reference in references {
            if let Some(url) = reference.get("url").and_then(|v| v.as_str()) {
                let ref_type = reference
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("osv");
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

/// Process OSV affected package
async fn process_osv_package(
    pool: &SqlitePool,
    vuln_id: i64,
    pkg: &serde_json::Value,
) -> Result<(), Error> {
    let package_name = pkg
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ecosystem = pkg
        .get("package")
        .and_then(|p| p.get("ecosystem"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let purl = pkg
        .get("package")
        .and_then(|p| p.get("purl"))
        .and_then(|v| v.as_str());

    if let Some(ranges) = pkg.get("ranges").and_then(|v| v.as_array()) {
        for range in ranges {
            if let Some(events) = range.get("events").and_then(|v| v.as_array()) {
                let mut affected_version = String::new();
                let mut fixed_version = String::new();

                for event in events {
                    if let Some(introduced) = event.get("introduced").and_then(|v| v.as_str()) {
                        affected_version = format!(">={introduced}");
                    }
                    if let Some(fixed) = event.get("fixed").and_then(|v| v.as_str()) {
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

    Ok(())
}

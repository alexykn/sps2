//! GitHub Security Advisory source implementation

use sps2_errors::{AuditError, Error};
use sqlx::SqlitePool;

/// Update from GitHub Security Advisories
pub(crate) async fn update_from_github(pool: &SqlitePool) -> Result<usize, Error> {
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
            .header("User-Agent", "sps2-package-manager")
            .header("Accept", "application/vnd.github.v4+json")
            .json(&serde_json::json!({ "query": query }))
            .send()
            .await
            .map_err(|e| AuditError::CveFetchError {
                message: format!("Failed to fetch from GitHub: {e}"),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AuditError::CveFetchError {
                message: format!("GitHub API returned status: {status}"),
            }
            .into());
        }

        let data: serde_json::Value =
            response
                .json()
                .await
                .map_err(|e| AuditError::CveFetchError {
                    message: format!("Failed to parse GitHub JSON: {e}"),
                })?;

        if let Some(advisories) = data
            .get("data")
            .and_then(|d| d.get("securityAdvisories"))
            .and_then(|sa| sa.get("nodes"))
            .and_then(|v| v.as_array())
        {
            for advisory in advisories {
                let count = insert_github_advisory(pool, advisory).await?;
                total_updated += count;
            }
        }

        // Check pagination
        has_next_page = data
            .get("data")
            .and_then(|d| d.get("securityAdvisories"))
            .and_then(|sa| sa.get("pageInfo"))
            .and_then(|pi| pi.get("hasNextPage"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        end_cursor = data
            .get("data")
            .and_then(|d| d.get("securityAdvisories"))
            .and_then(|sa| sa.get("pageInfo"))
            .and_then(|pi| pi.get("endCursor"))
            .and_then(|v| v.as_str())
            .map(String::from);

        if has_next_page {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    Ok(total_updated)
}

/// Insert GitHub advisory into database
async fn insert_github_advisory(
    pool: &SqlitePool,
    advisory: &serde_json::Value,
) -> Result<usize, Error> {
    let ghsa_id = advisory
        .get("ghsaId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if ghsa_id.is_empty() {
        return Ok(0);
    }

    let summary = advisory
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("No summary");
    let severity = advisory
        .get("severity")
        .and_then(|v| v.as_str())
        .unwrap_or("medium")
        .to_lowercase();

    let published = advisory
        .get("publishedAt")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");
    let updated = advisory
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .unwrap_or(published);

    // Extract CVSS score if available
    let cvss_score = advisory
        .get("cvss")
        .and_then(|cvss| cvss.get("score"))
        .and_then(serde_json::Value::as_f64)
        .map(|s| s as f32);

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
    process_github_vulnerabilities(pool, vuln_id, advisory).await?;

    // Insert references
    process_github_references(pool, vuln_id, advisory).await?;

    Ok(1)
}

/// Process vulnerabilities from GitHub advisory
async fn process_github_vulnerabilities(
    pool: &SqlitePool,
    vuln_id: i64,
    advisory: &serde_json::Value,
) -> Result<(), Error> {
    if let Some(vulnerabilities) = advisory
        .get("vulnerabilities")
        .and_then(|v| v.get("nodes"))
        .and_then(|n| n.as_array())
    {
        for vuln in vulnerabilities {
            let package_name = vuln
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ecosystem = vuln
                .get("package")
                .and_then(|p| p.get("ecosystem"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let affected_range = vuln
                .get("vulnerableVersionRange")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let first_patched = vuln
                .get("firstPatchedVersion")
                .and_then(|fpv| fpv.get("identifier"))
                .and_then(|v| v.as_str())
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
    Ok(())
}

/// Process references from GitHub advisory
async fn process_github_references(
    pool: &SqlitePool,
    vuln_id: i64,
    advisory: &serde_json::Value,
) -> Result<(), Error> {
    if let Some(references) = advisory
        .get("references")
        .and_then(|r| r.get("nodes"))
        .and_then(|n| n.as_array())
    {
        for reference in references {
            if let Some(url) = reference.get("url").and_then(|v| v.as_str()) {
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
    Ok(())
}

/// Build GitHub GraphQL query for security advisories
fn build_github_query(cursor: Option<&str>) -> String {
    let after = cursor
        .map(|c| format!(r#", after: "{c}""#))
        .unwrap_or_default();

    format!(
        r"
        query {{
            securityAdvisories(first: 100{after}) {{
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
        "
    )
}

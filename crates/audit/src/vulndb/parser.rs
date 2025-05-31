//! Shared parsing utilities for vulnerability data

/// Check if a version is affected by vulnerability
pub(crate) fn is_version_affected(
    version: &str,
    affected_range: &str,
    fixed_version: Option<&str>,
) -> bool {
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
pub(crate) fn extract_nvd_severity(
    cve: &serde_json::Map<String, serde_json::Value>,
) -> (&'static str, Option<f32>) {
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

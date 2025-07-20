//! Shared parsing utilities for vulnerability data

/// Check if a version is affected by vulnerability using proper semver parsing
pub(crate) fn is_version_affected(
    version: &str,
    affected_range: &str,
    fixed_version: Option<&str>,
) -> bool {
    // Try to parse the version as semver first
    let parsed_version = match semver::Version::parse(version) {
        Ok(v) => v,
        Err(_) => {
            // If semver parsing fails, try to parse as loose version
            match semver::Version::parse(&normalize_version(version)) {
                Ok(v) => v,
                Err(_) => {
                    // If all parsing fails, fall back to string comparison for safety
                    return string_version_affected(version, affected_range, fixed_version);
                }
            }
        }
    };

    // Handle empty or wildcard ranges
    if affected_range == "*" || affected_range.is_empty() {
        return if let Some(fixed) = fixed_version {
            check_version_less_than(&parsed_version, fixed)
        } else {
            true
        };
    }

    // Parse version range specifiers
    if let Some(range) = parse_version_range(affected_range) {
        let is_in_range = match range {
            VersionRange::GreaterEqual(min_ver) => parsed_version >= min_ver,
            VersionRange::LessEqual(max_ver) => parsed_version <= max_ver,
            VersionRange::Less(max_ver) => parsed_version < max_ver,
            VersionRange::Greater(min_ver) => parsed_version > min_ver,
            VersionRange::Equal(exact_ver) => parsed_version == exact_ver,
            VersionRange::Tilde(base_ver) => {
                // ~1.2.3 := >=1.2.3 <1.3.0 (reasonably close to 1.2.3)
                let next_minor = semver::Version::new(base_ver.major, base_ver.minor + 1, 0);
                parsed_version >= base_ver && parsed_version < next_minor
            }
            VersionRange::Caret(base_ver) => {
                // ^1.2.3 := >=1.2.3 <2.0.0 (compatible within same major version)
                let next_major = semver::Version::new(base_ver.major + 1, 0, 0);
                parsed_version >= base_ver && parsed_version < next_major
            }
        };

        // If in affected range, check if there's a fix version
        if is_in_range {
            if let Some(fixed) = fixed_version {
                check_version_less_than(&parsed_version, fixed)
            } else {
                true
            }
        } else {
            false
        }
    } else {
        // For complex ranges that we can't parse, fall back to string comparison
        string_version_affected(version, affected_range, fixed_version)
    }
}

/// Version range types supported in vulnerability specifications
#[derive(Debug, Clone, PartialEq)]
enum VersionRange {
    GreaterEqual(semver::Version),
    LessEqual(semver::Version),
    Less(semver::Version),
    Greater(semver::Version),
    Equal(semver::Version),
    Tilde(semver::Version), // ~1.2.3
    Caret(semver::Version), // ^1.2.3
}

/// Parse a version range specifier into a structured format
fn parse_version_range(range_str: &str) -> Option<VersionRange> {
    let trimmed = range_str.trim();

    if trimmed.starts_with(">=") {
        let version_str = trimmed.trim_start_matches(">=").trim();
        semver::Version::parse(&normalize_version(version_str))
            .ok()
            .map(VersionRange::GreaterEqual)
    } else if trimmed.starts_with("<=") {
        let version_str = trimmed.trim_start_matches("<=").trim();
        semver::Version::parse(&normalize_version(version_str))
            .ok()
            .map(VersionRange::LessEqual)
    } else if trimmed.starts_with('<') {
        let version_str = trimmed.trim_start_matches('<').trim();
        semver::Version::parse(&normalize_version(version_str))
            .ok()
            .map(VersionRange::Less)
    } else if trimmed.starts_with('>') {
        let version_str = trimmed.trim_start_matches('>').trim();
        semver::Version::parse(&normalize_version(version_str))
            .ok()
            .map(VersionRange::Greater)
    } else if trimmed.starts_with('~') {
        let version_str = trimmed.trim_start_matches('~').trim();
        semver::Version::parse(&normalize_version(version_str))
            .ok()
            .map(VersionRange::Tilde)
    } else if trimmed.starts_with('^') {
        let version_str = trimmed.trim_start_matches('^').trim();
        semver::Version::parse(&normalize_version(version_str))
            .ok()
            .map(VersionRange::Caret)
    } else if trimmed.starts_with('=') {
        let version_str = trimmed.trim_start_matches('=').trim();
        semver::Version::parse(&normalize_version(version_str))
            .ok()
            .map(VersionRange::Equal)
    } else {
        // Try to parse as exact version
        semver::Version::parse(&normalize_version(trimmed))
            .ok()
            .map(VersionRange::Equal)
    }
}

/// Normalize a version string to be semver-compatible
fn normalize_version(version: &str) -> String {
    let trimmed = version.trim();

    // Handle common version formats
    if trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
        let parts: Vec<&str> = trimmed.split('.').collect();
        match parts.len() {
            1 => format!("{}.0.0", parts[0]),
            2 => format!("{}.{}.0", parts[0], parts[1]),
            _ => trimmed.to_string(),
        }
    } else {
        trimmed.to_string()
    }
}

/// Check if a version is less than another version string
fn check_version_less_than(version: &semver::Version, fixed_str: &str) -> bool {
    match semver::Version::parse(&normalize_version(fixed_str)) {
        Ok(fixed_version) => *version < fixed_version,
        Err(_) => {
            // Fall back to string comparison if semver parsing fails
            version.to_string().as_str() < fixed_str
        }
    }
}

/// Fallback string-based version comparison for non-semver versions
fn string_version_affected(
    version: &str,
    affected_range: &str,
    fixed_version: Option<&str>,
) -> bool {
    if affected_range == "*" || affected_range.is_empty() {
        if let Some(fixed) = fixed_version {
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
            let score = metric["cvssData"]["baseScore"].as_f64().map(|s| {
                // CVSS scores are 0.0-10.0, so f32 precision is sufficient
                #[allow(clippy::cast_possible_truncation)]
                {
                    s as f32
                }
            });

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
            let score = metric["cvssData"]["baseScore"].as_f64().map(|s| {
                // CVSS scores are 0.0-10.0, so f32 precision is sufficient
                #[allow(clippy::cast_possible_truncation)]
                {
                    s as f32
                }
            });
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

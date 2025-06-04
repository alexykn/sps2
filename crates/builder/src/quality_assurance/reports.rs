//! Quality assurance reporting

use super::types::{QaCheck, QaCheckType, QaSeverity};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Quality assurance report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaReport {
    /// Report ID
    pub id: String,
    /// Package name
    pub package_name: String,
    /// Package version
    pub package_version: String,
    /// Timestamp when the report was generated
    pub timestamp: DateTime<Utc>,
    /// Duration of the QA checks in seconds
    pub duration_seconds: u64,
    /// All QA check results
    pub checks: Vec<QaCheck>,
    /// Summary statistics
    pub summary: QaSummary,
    /// Metadata about the QA run
    pub metadata: HashMap<String, String>,
}

/// Summary of QA results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaSummary {
    /// Total number of checks performed
    pub total_checks: usize,
    /// Number of passed checks
    pub passed: usize,
    /// Number of failed checks
    pub failed: usize,
    /// Number of checks by severity
    pub by_severity: HashMap<QaSeverity, usize>,
    /// Number of checks by type
    pub by_type: HashMap<QaCheckType, usize>,
    /// Overall status
    pub status: QaStatus,
}

/// Overall QA status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QaStatus {
    /// All checks passed
    Passed,
    /// Has warnings but no errors
    PassedWithWarnings,
    /// Has errors
    Failed,
}

/// Result of a QA pipeline execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaResult {
    /// Whether the check succeeded
    pub success: bool,
    /// Check results
    pub checks: Vec<QaCheck>,
    /// Execution time in milliseconds
    pub duration_ms: u64,
}

impl QaReport {
    /// Create a new QA report
    #[must_use]
    pub fn new(
        package_name: impl Into<String>,
        package_version: impl Into<String>,
        checks: Vec<QaCheck>,
        duration_seconds: u64,
    ) -> Self {
        let summary = Self::calculate_summary(&checks);

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            package_name: package_name.into(),
            package_version: package_version.into(),
            timestamp: Utc::now(),
            duration_seconds,
            checks,
            summary,
            metadata: HashMap::new(),
        }
    }

    /// Calculate summary statistics from checks
    fn calculate_summary(checks: &[QaCheck]) -> QaSummary {
        let total_checks = checks.len();
        let mut by_severity: HashMap<QaSeverity, usize> = HashMap::new();
        let mut by_type: HashMap<QaCheckType, usize> = HashMap::new();
        let mut failed = 0;

        for check in checks {
            *by_severity.entry(check.severity).or_insert(0) += 1;
            *by_type.entry(check.check_type).or_insert(0) += 1;

            if matches!(check.severity, QaSeverity::Error | QaSeverity::Critical) {
                failed += 1;
            }
        }

        let passed = total_checks - failed;
        let has_warnings = by_severity.get(&QaSeverity::Warning).copied().unwrap_or(0) > 0;

        let status = if failed > 0 {
            QaStatus::Failed
        } else if has_warnings {
            QaStatus::PassedWithWarnings
        } else {
            QaStatus::Passed
        };

        QaSummary {
            total_checks,
            passed,
            failed,
            by_severity,
            by_type,
            status,
        }
    }

    /// Check if the report has any errors
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.summary.failed > 0
    }

    /// Check if the report has any warnings
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        self.summary
            .by_severity
            .get(&QaSeverity::Warning)
            .copied()
            .unwrap_or(0)
            > 0
    }

    /// Get the number of errors
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.summary
            .by_severity
            .get(&QaSeverity::Error)
            .copied()
            .unwrap_or(0)
            + self
                .summary
                .by_severity
                .get(&QaSeverity::Critical)
                .copied()
                .unwrap_or(0)
    }

    /// Get the number of warnings
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.summary
            .by_severity
            .get(&QaSeverity::Warning)
            .copied()
            .unwrap_or(0)
    }

    /// Get total number of checks
    #[must_use]
    pub fn total_checks(&self) -> usize {
        self.summary.total_checks
    }

    /// Add metadata to the report
    pub fn add_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }

    /// Format report as human-readable text
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!(
            "Quality Assurance Report for {} v{}\n",
            self.package_name, self.package_version
        ));
        output.push_str(&format!(
            "Generated: {}\n",
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        output.push_str(&format!("Duration: {}s\n\n", self.duration_seconds));

        output.push_str("Summary:\n");
        output.push_str(&format!("  Total Checks: {}\n", self.summary.total_checks));
        output.push_str(&format!("  Passed: {}\n", self.summary.passed));
        output.push_str(&format!("  Failed: {}\n", self.summary.failed));
        output.push_str(&format!("  Status: {:?}\n\n", self.summary.status));

        if !self.checks.is_empty() {
            output.push_str("Findings:\n");

            // Group by severity
            let mut by_severity: HashMap<QaSeverity, Vec<&QaCheck>> = HashMap::new();
            for check in &self.checks {
                by_severity.entry(check.severity).or_default().push(check);
            }

            // Output in order of severity
            for severity in [
                QaSeverity::Critical,
                QaSeverity::Error,
                QaSeverity::Warning,
                QaSeverity::Info,
            ] {
                if let Some(checks) = by_severity.get(&severity) {
                    output.push_str(&format!("\n{} ({}):\n", severity, checks.len()));
                    for check in checks {
                        output.push_str(&format!(
                            "  - [{}] {}: {}\n",
                            check.check_type, check.check_name, check.message
                        ));

                        if let Some(path) = &check.file_path {
                            output.push_str(&format!("    File: {}", path.display()));
                            if let Some(line) = check.line_number {
                                output.push_str(&format!(":{}", line));
                                if let Some(col) = check.column_number {
                                    output.push_str(&format!(":{}", col));
                                }
                            }
                            output.push('\n');
                        }

                        if let Some(code) = &check.code {
                            output.push_str(&format!("    Code: {}\n", code));
                        }

                        if let Some(context) = &check.context {
                            output.push_str(&format!("    Context: {}\n", context));
                        }
                    }
                }
            }
        }

        output
    }

    /// Format report as SARIF (Static Analysis Results Interchange Format)
    #[must_use]
    pub fn to_sarif(&self) -> serde_json::Value {
        // SARIF 2.1.0 format
        serde_json::json!({
            "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
            "version": "2.1.0",
            "runs": [{
                "tool": {
                    "driver": {
                        "name": "sps2-qa",
                        "version": env!("CARGO_PKG_VERSION"),
                        "informationUri": "https://github.com/yourusername/sps2",
                        "rules": self.generate_sarif_rules()
                    }
                },
                "results": self.generate_sarif_results(),
                "invocations": [{
                    "executionSuccessful": !self.has_errors(),
                    "endTimeUtc": self.timestamp.to_rfc3339(),
                }]
            }]
        })
    }

    fn generate_sarif_rules(&self) -> Vec<serde_json::Value> {
        let mut rules = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for check in &self.checks {
            let rule_id = check.code.as_ref().unwrap_or(&check.check_name);
            if seen.insert(rule_id.clone()) {
                rules.push(serde_json::json!({
                    "id": rule_id,
                    "name": check.check_name,
                    "defaultConfiguration": {
                        "level": match check.severity {
                            QaSeverity::Critical | QaSeverity::Error => "error",
                            QaSeverity::Warning => "warning",
                            QaSeverity::Info => "note",
                        }
                    }
                }));
            }
        }

        rules
    }

    fn generate_sarif_results(&self) -> Vec<serde_json::Value> {
        self.checks
            .iter()
            .map(|check| {
                let mut result = serde_json::json!({
                    "ruleId": check.code.as_ref().unwrap_or(&check.check_name),
                    "level": match check.severity {
                        QaSeverity::Critical | QaSeverity::Error => "error",
                        QaSeverity::Warning => "warning",
                        QaSeverity::Info => "note",
                    },
                    "message": {
                        "text": &check.message
                    }
                });

                if let Some(path) = &check.file_path {
                    let location = serde_json::json!({
                        "physicalLocation": {
                            "artifactLocation": {
                                "uri": path.display().to_string()
                            },
                            "region": {
                                "startLine": check.line_number.unwrap_or(1),
                                "startColumn": check.column_number.unwrap_or(1)
                            }
                        }
                    });
                    result["locations"] = serde_json::json!([location]);
                }

                result
            })
            .collect()
    }

    /// Format report as JUnit XML
    #[must_use]
    pub fn to_junit(&self) -> String {
        let mut xml = String::new();
        xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        xml.push('\n');

        xml.push_str(&format!(
            r#"<testsuites name="sps2-qa" tests="{}" failures="{}" errors="{}" time="{}">"#,
            self.summary.total_checks,
            self.warning_count(),
            self.error_count(),
            self.duration_seconds
        ));
        xml.push('\n');

        // Group checks by type
        let mut by_type: HashMap<QaCheckType, Vec<&QaCheck>> = HashMap::new();
        for check in &self.checks {
            by_type.entry(check.check_type).or_default().push(check);
        }

        for (check_type, checks) in by_type {
            let failures = checks
                .iter()
                .filter(|c| c.severity == QaSeverity::Warning)
                .count();
            let errors = checks
                .iter()
                .filter(|c| matches!(c.severity, QaSeverity::Error | QaSeverity::Critical))
                .count();

            xml.push_str(&format!(
                r#"  <testsuite name="{}" tests="{}" failures="{}" errors="{}">"#,
                check_type,
                checks.len(),
                failures,
                errors
            ));
            xml.push('\n');

            for check in checks {
                xml.push_str(&format!(
                    r#"    <testcase name="{}" classname="{}">"#,
                    check.check_name, check.check_type
                ));

                match check.severity {
                    QaSeverity::Error | QaSeverity::Critical => {
                        xml.push('\n');
                        xml.push_str(&format!(
                            r#"      <error message="{}" type="{}"/>"#,
                            xml_escape(&check.message),
                            check.severity
                        ));
                        xml.push('\n');
                        xml.push_str("    </testcase>");
                    }
                    QaSeverity::Warning => {
                        xml.push('\n');
                        xml.push_str(&format!(
                            r#"      <failure message="{}" type="{}"/>"#,
                            xml_escape(&check.message),
                            check.severity
                        ));
                        xml.push('\n');
                        xml.push_str("    </testcase>");
                    }
                    QaSeverity::Info => {
                        xml.push_str("/>");
                    }
                }
                xml.push('\n');
            }

            xml.push_str("  </testsuite>\n");
        }

        xml.push_str("</testsuites>\n");
        xml
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

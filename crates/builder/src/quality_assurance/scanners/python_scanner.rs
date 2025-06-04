//! Python security scanners (Bandit)

use super::{run_scanner_command, Scanner};
use crate::quality_assurance::types::{QaCheck, QaCheckType, QaSeverity, ScannerConfig};
use crate::BuildContext;
use serde::Deserialize;
use sps2_errors::Error;
use std::path::Path;

/// Bandit security scanner for Python code
pub struct BanditScanner;

impl BanditScanner {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for BanditScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Scanner for BanditScanner {
    fn name(&self) -> &'static str {
        "bandit"
    }

    fn can_handle(&self, path: &Path) -> bool {
        // Check for Python files
        if path.is_file() {
            return path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e, "py" | "pyi"))
                .unwrap_or(false);
        }

        // Check if directory contains Python files
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                    if matches!(ext, "py" | "pyi") {
                        return true;
                    }
                }
            }
        }

        false
    }

    async fn scan(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &ScannerConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut args = vec!["-r".to_string(), "-f".to_string(), "json".to_string()];
        args.extend(config.args.clone());
        args.push(path.display().to_string());

        let output = run_scanner_command(&config.command, &args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Parse JSON output
        if let Ok(bandit_result) = serde_json::from_slice::<BanditOutput>(&output.stdout) {
            for result in &bandit_result.results {
                let severity = map_bandit_severity(&result.issue_severity);

                let check = QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "bandit",
                    severity,
                    format!("[{}] {}", result.test_id, result.issue_text),
                )
                .with_location(
                    Path::new(&result.filename).to_path_buf(),
                    Some(result.line_number),
                    Some(result.col_offset),
                )
                .with_code(&result.test_id)
                .with_context(format!(
                    "Confidence: {}\nCWE: {}\nMore info: {}",
                    result.issue_confidence,
                    result
                        .issue_cwe
                        .get("id")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    result.more_info.as_deref().unwrap_or("")
                ));

                checks.push(check);
            }

            // Check metrics for overall security score
            if let Some(metrics) = &bandit_result.metrics {
                let total_issues = metrics
                    .get("SEVERITY.HIGH")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
                    + metrics
                        .get("SEVERITY.MEDIUM")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);

                if total_issues > 0 && checks.is_empty() {
                    checks.push(QaCheck::new(
                        QaCheckType::SecurityScanner,
                        "bandit",
                        QaSeverity::Warning,
                        format!("Found {} security issues", total_issues),
                    ));
                }
            }
        } else {
            // Fallback to stderr parsing
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("security issue") {
                checks.push(QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "bandit",
                    QaSeverity::Warning,
                    "Security issues detected (run bandit for details)",
                ));
            }
        }

        Ok(checks)
    }
}

fn map_bandit_severity(severity: &str) -> QaSeverity {
    match severity.to_uppercase().as_str() {
        "HIGH" => QaSeverity::Critical,
        "MEDIUM" => QaSeverity::Error,
        "LOW" => QaSeverity::Warning,
        _ => QaSeverity::Info,
    }
}

// Bandit JSON output structures
#[derive(Deserialize)]
struct BanditOutput {
    results: Vec<BanditResult>,
    metrics: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Deserialize)]
struct BanditResult {
    filename: String,
    line_number: usize,
    col_offset: usize,
    test_id: String,
    issue_text: String,
    issue_severity: String,
    issue_confidence: String,
    issue_cwe: serde_json::Map<String, serde_json::Value>,
    more_info: Option<String>,
}

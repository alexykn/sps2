//! NPM audit scanner for Node.js dependencies

use super::{map_severity_string, run_scanner_command, Scanner};
use crate::quality_assurance::types::{QaCheck, QaCheckType, QaSeverity, ScannerConfig};
use crate::BuildContext;
use serde::Deserialize;
use sps2_errors::Error;
use std::collections::HashMap;
use std::path::Path;

/// NPM audit scanner for Node.js projects
pub struct NpmAuditScanner;

impl NpmAuditScanner {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NpmAuditScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Scanner for NpmAuditScanner {
    fn name(&self) -> &'static str {
        "npm-audit"
    }

    fn can_handle(&self, path: &Path) -> bool {
        // Check for package-lock.json
        path.join("package-lock.json").exists() || 
        // Check for package.json (might not have lock file yet)
        path.join("package.json").exists()
    }

    async fn scan(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &ScannerConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        // First check if package-lock.json exists, if not we need to run npm install
        if !path.join("package-lock.json").exists() && path.join("package.json").exists() {
            // Run npm install to generate lock file
            let install_output = run_scanner_command(
                "npm",
                &["install".to_string(), "--package-lock-only".to_string()],
                path,
                &config.env,
            )
            .await?;

            if !install_output.status.success() {
                return Ok(vec![QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "npm-audit",
                    QaSeverity::Warning,
                    "Failed to generate package-lock.json for audit",
                )]);
            }
        }

        let mut args = vec!["audit".to_string(), "--json".to_string()];
        args.extend(config.args.clone());

        let output = run_scanner_command(&config.command, &args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Parse JSON output - npm audit always returns non-zero on vulnerabilities
        if let Ok(audit_result) = serde_json::from_slice::<NpmAuditOutput>(&output.stdout) {
            // Process advisories
            for advisory in audit_result.advisories.values() {
                let severity = map_severity_string(&advisory.severity);

                let check = QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "npm-audit",
                    severity,
                    format!(
                        "{}: {} in {}",
                        advisory.title, advisory.module_name, advisory.vulnerable_versions
                    ),
                )
                .with_code(format!("npm-{}", advisory.id))
                .with_context(format!(
                    "Overview: {}\nRecommendation: {}\nPatched versions: {}",
                    advisory.overview,
                    advisory.recommendation,
                    advisory.patched_versions.as_deref().unwrap_or("none")
                ));

                checks.push(check);
            }

            // If no advisories but we have metadata, check severity counts
            if checks.is_empty() && audit_result.metadata.vulnerabilities.total > 0 {
                let meta = &audit_result.metadata.vulnerabilities;
                checks.push(QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "npm-audit",
                    if meta.critical > 0 {
                        QaSeverity::Critical
                    } else if meta.high > 0 {
                        QaSeverity::Error
                    } else if meta.moderate > 0 {
                        QaSeverity::Warning
                    } else {
                        QaSeverity::Info
                    },
                    format!(
                        "Found {} vulnerabilities (critical: {}, high: {}, moderate: {}, low: {})",
                        meta.total, meta.critical, meta.high, meta.moderate, meta.low
                    ),
                ));
            }
        } else {
            // Try newer npm audit format
            if let Ok(new_format) = serde_json::from_slice::<NpmAuditNewFormat>(&output.stdout) {
                for vuln in new_format.vulnerabilities.values() {
                    let severity = map_severity_string(&vuln.severity);

                    let check = QaCheck::new(
                        QaCheckType::SecurityScanner,
                        "npm-audit",
                        severity,
                        format!("{} in {}", vuln.title, vuln.name),
                    )
                    .with_context(format!(
                        "Via: {}\nRange: {}\nFixed in: {}",
                        vuln.via
                            .iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        vuln.range,
                        vuln.fixed_in.as_deref().unwrap_or("No fix available")
                    ));

                    checks.push(check);
                }
            } else {
                // Fallback for parsing errors
                if !output.status.success() {
                    checks.push(QaCheck::new(
                        QaCheckType::SecurityScanner,
                        "npm-audit",
                        QaSeverity::Warning,
                        "npm audit found vulnerabilities (run npm audit for details)",
                    ));
                }
            }
        }

        Ok(checks)
    }
}

// NPM audit JSON output structures (older format)
#[derive(Deserialize)]
struct NpmAuditOutput {
    advisories: HashMap<String, Advisory>,
    metadata: Metadata,
}

#[derive(Deserialize)]
struct Advisory {
    id: u64,
    title: String,
    module_name: String,
    severity: String,
    vulnerable_versions: String,
    patched_versions: Option<String>,
    overview: String,
    recommendation: String,
}

#[derive(Deserialize)]
struct Metadata {
    vulnerabilities: VulnerabilityCounts,
}

#[derive(Deserialize)]
struct VulnerabilityCounts {
    total: u64,
    critical: u64,
    high: u64,
    moderate: u64,
    low: u64,
}

// NPM audit JSON output structures (newer format)
#[derive(Deserialize)]
struct NpmAuditNewFormat {
    vulnerabilities: HashMap<String, Vulnerability>,
}

#[derive(Deserialize)]
struct Vulnerability {
    name: String,
    severity: String,
    via: Vec<serde_json::Value>,
    range: String,
    title: String,
    fixed_in: Option<String>,
}

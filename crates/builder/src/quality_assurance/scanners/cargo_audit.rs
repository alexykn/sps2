//! Cargo audit scanner for Rust dependencies

use super::{map_severity_string, run_scanner_command, Scanner};
use crate::quality_assurance::types::{QaCheck, QaCheckType, QaSeverity, ScannerConfig};
use crate::BuildContext;
use serde::Deserialize;
use sps2_errors::Error;
use std::path::Path;

/// Cargo audit scanner for Rust projects
pub struct CargoAuditScanner;

impl CargoAuditScanner {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CargoAuditScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Scanner for CargoAuditScanner {
    fn name(&self) -> &'static str {
        "cargo-audit"
    }

    fn can_handle(&self, path: &Path) -> bool {
        // Check for Cargo.lock file
        path.join("Cargo.lock").exists() || 
        // Check in parent directories
        {
            let mut current = path;
            loop {
                if current.join("Cargo.lock").exists() {
                    return true;
                }
                match current.parent() {
                    Some(parent) => current = parent,
                    None => return false,
                }
            }
        }
    }

    async fn scan(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &ScannerConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut args = vec!["audit".to_string(), "--json".to_string()];
        args.extend(config.args.clone());

        let output = run_scanner_command(&config.command, &args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Parse JSON output
        if let Ok(audit_result) = serde_json::from_slice::<CargoAuditOutput>(&output.stdout) {
            // Process vulnerabilities
            for vuln in &audit_result.vulnerabilities.list {
                if let Some(advisory) = &vuln.advisory {
                    let severity = map_severity_string(&advisory.severity);

                    let check = QaCheck::new(
                        QaCheckType::SecurityScanner,
                        "cargo-audit",
                        severity,
                        format!(
                            "{}: {} ({})",
                            advisory.id, advisory.title, vuln.package.name
                        ),
                    )
                    .with_code(&advisory.id)
                    .with_context(format!(
                        "Package: {} v{}\nCVE: {}\nDescription: {}",
                        vuln.package.name,
                        vuln.package.version,
                        advisory.cve.as_deref().unwrap_or("N/A"),
                        advisory.description
                    ));

                    checks.push(check);
                }
            }

            // Process warnings
            for warning in &audit_result.warnings {
                let check = QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "cargo-audit",
                    QaSeverity::Warning,
                    format!("{}: {}", warning.kind, warning.message),
                );

                checks.push(check);
            }
        } else {
            // Fallback to text parsing if JSON fails
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("vulnerabilities found") {
                checks.push(QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "cargo-audit",
                    QaSeverity::Error,
                    "Vulnerabilities detected (run cargo audit for details)",
                ));
            }
        }

        Ok(checks)
    }
}

// Cargo audit JSON output structures
#[derive(Deserialize)]
struct CargoAuditOutput {
    vulnerabilities: Vulnerabilities,
    warnings: Vec<Warning>,
}

#[derive(Deserialize)]
struct Vulnerabilities {
    list: Vec<Vulnerability>,
}

#[derive(Deserialize)]
struct Vulnerability {
    advisory: Option<Advisory>,
    package: Package,
}

#[derive(Deserialize)]
struct Advisory {
    id: String,
    title: String,
    description: String,
    severity: String,
    cve: Option<String>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct Warning {
    kind: String,
    message: String,
}

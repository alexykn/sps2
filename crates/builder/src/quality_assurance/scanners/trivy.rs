//! Trivy vulnerability scanner for multiple languages and containers

use super::{map_cvss_to_severity, map_severity_string, run_scanner_command, Scanner};
use crate::quality_assurance::types::{QaCheck, QaCheckType, QaSeverity, ScannerConfig};
use crate::BuildContext;
use serde::Deserialize;
use sps2_errors::Error;
use std::path::Path;

/// Trivy scanner for comprehensive vulnerability scanning
pub struct TrivyScanner;

impl TrivyScanner {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for TrivyScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Scanner for TrivyScanner {
    fn name(&self) -> &'static str {
        "trivy"
    }

    fn can_handle(&self, _path: &Path) -> bool {
        // Trivy can scan any directory
        true
    }

    async fn scan(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &ScannerConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let output = self.run_trivy_scan(path, config).await?;
        let mut checks = Vec::new();

        // Parse JSON output
        if let Ok(trivy_result) = serde_json::from_slice::<TrivyOutput>(&output.stdout) {
            for result in &trivy_result.results {
                self.process_vulnerabilities(result, &mut checks);
                self.process_misconfigurations(result, &mut checks);
            }
        } else {
            self.fallback_check(&output, &mut checks);
        }

        Ok(checks)
    }
}

impl TrivyScanner {
    async fn run_trivy_scan(
        &self,
        path: &Path,
        config: &ScannerConfig,
    ) -> Result<std::process::Output, Error> {
        let mut args = vec![
            "fs".to_string(),
            "--format".to_string(),
            "json".to_string(),
            "--quiet".to_string(),
        ];
        args.extend(config.args.clone());
        args.push(path.display().to_string());

        run_scanner_command(&config.command, &args, path, &config.env).await
    }

    fn process_vulnerabilities(&self, result: &TrivyResult, checks: &mut Vec<QaCheck>) {
        if let Some(vulnerabilities) = &result.vulnerabilities {
            for vuln in vulnerabilities {
                let severity = vuln
                    .severity
                    .as_ref()
                    .map(|s| map_severity_string(s))
                    .or_else(|| {
                        vuln.cvss
                            .as_ref()
                            .and_then(|cvss| {
                                cvss.nvd
                                    .as_ref()
                                    .and_then(|nvd| nvd.v3_score)
                                    .or(cvss.nvd.as_ref().and_then(|nvd| nvd.v2_score))
                            })
                            .map(map_cvss_to_severity)
                    })
                    .unwrap_or(QaSeverity::Warning);

                let check = QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "trivy",
                    severity,
                    format!(
                        "{}: {} in {} {}",
                        vuln.id,
                        vuln.title.as_deref().unwrap_or("Vulnerability"),
                        vuln.pkg_name,
                        vuln.installed_version
                    ),
                )
                .with_code(&vuln.id)
                .with_context(format!(
                    "Type: {}\nFixed Version: {}\nDescription: {}",
                    result.target_type.as_deref().unwrap_or("unknown"),
                    vuln.fixed_version.as_deref().unwrap_or("none"),
                    vuln.description
                        .as_deref()
                        .unwrap_or("No description available")
                ));

                if let Some(target) = &result.target {
                    checks.push(check.with_location(Path::new(target).to_path_buf(), None, None));
                } else {
                    checks.push(check);
                }
            }
        }
    }

    fn process_misconfigurations(&self, result: &TrivyResult, checks: &mut Vec<QaCheck>) {
        if let Some(misconfigs) = &result.misconfigurations {
            for misconfig in misconfigs {
                let severity = map_severity_string(&misconfig.severity);

                let check = QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "trivy",
                    severity,
                    format!("{}: {}", misconfig.id, misconfig.title),
                )
                .with_code(&misconfig.id)
                .with_context(format!(
                    "Type: {}\nDescription: {}\nResolution: {}",
                    misconfig.misconfig_type,
                    misconfig.description,
                    misconfig
                        .resolution
                        .as_deref()
                        .unwrap_or("No resolution provided")
                ));

                if let Some(target) = &result.target {
                    checks.push(check.with_location(Path::new(target).to_path_buf(), None, None));
                } else {
                    checks.push(check);
                }
            }
        }
    }

    fn fallback_check(&self, output: &std::process::Output, checks: &mut Vec<QaCheck>) {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("vulnerability")
                || stderr.contains("CRITICAL")
                || stderr.contains("HIGH")
            {
                checks.push(QaCheck::new(
                    QaCheckType::SecurityScanner,
                    "trivy",
                    QaSeverity::Error,
                    "Security vulnerabilities detected (run trivy for details)",
                ));
            }
        }
    }
}

// Trivy JSON output structures
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TrivyOutput {
    results: Vec<TrivyResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TrivyResult {
    target: Option<String>,
    #[serde(rename = "Type")]
    target_type: Option<String>,
    vulnerabilities: Option<Vec<Vulnerability>>,
    misconfigurations: Option<Vec<Misconfiguration>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Vulnerability {
    id: String,
    pkg_name: String,
    installed_version: String,
    fixed_version: Option<String>,
    title: Option<String>,
    description: Option<String>,
    severity: Option<String>,
    #[serde(rename = "CVSS")]
    cvss: Option<CvssScores>,
}

#[derive(Deserialize)]
struct CvssScores {
    nvd: Option<NvdScore>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NvdScore {
    #[serde(rename = "V3Score")]
    v3_score: Option<f64>,
    #[serde(rename = "V2Score")]
    v2_score: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Misconfiguration {
    id: String,
    #[serde(rename = "Type")]
    misconfig_type: String,
    title: String,
    description: String,
    severity: String,
    resolution: Option<String>,
}

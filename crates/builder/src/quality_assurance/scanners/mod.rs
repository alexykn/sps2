//! Security vulnerability scanning for packages

pub mod cargo_audit;
pub mod npm_audit;
pub mod python_scanner;
pub mod trivy;

use super::types::{QaCheck, QaCheckType, QaSeverity, ScannerConfig};
use crate::events::send_event;
use crate::BuildContext;
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use std::collections::HashMap;
use std::path::Path;
use tokio::process::Command;

/// Scanner trait for implementing security scanners
#[async_trait::async_trait]
pub trait Scanner: Send + Sync {
    /// Name of the scanner
    fn name(&self) -> &str;

    /// Check if this scanner can handle the given path
    fn can_handle(&self, path: &Path) -> bool;

    /// Run the scanner on the given path
    async fn scan(
        &self,
        context: &BuildContext,
        path: &Path,
        config: &ScannerConfig,
    ) -> Result<Vec<QaCheck>, Error>;
}

/// Scanner registry managing all available scanners
pub struct ScannerRegistry {
    scanners: HashMap<String, Box<dyn Scanner>>,
}

impl ScannerRegistry {
    /// Create a new scanner registry with all built-in scanners
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            scanners: HashMap::new(),
        };

        // Register built-in scanners
        registry.register(Box::new(cargo_audit::CargoAuditScanner::new()));
        registry.register(Box::new(trivy::TrivyScanner::new()));
        registry.register(Box::new(python_scanner::BanditScanner::new()));
        registry.register(Box::new(npm_audit::NpmAuditScanner::new()));

        registry
    }

    /// Register a custom scanner
    pub fn register(&mut self, scanner: Box<dyn Scanner>) {
        self.scanners.insert(scanner.name().to_string(), scanner);
    }

    /// Get a scanner by name
    pub fn get(&self, name: &str) -> Option<&dyn Scanner> {
        self.scanners.get(name).map(std::convert::AsRef::as_ref)
    }

    /// Run all applicable scanners on a directory
    pub async fn scan_directory(
        &self,
        context: &BuildContext,
        dir: &Path,
        configs: &HashMap<String, ScannerConfig>,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut all_checks = Vec::new();

        for (name, config) in configs {
            if !config.enabled {
                continue;
            }

            if let Some(scanner) = self.get(name) {
                send_event(
                    context,
                    Event::OperationStarted {
                        operation: format!("Running {} security scanner", name),
                    },
                );

                match scanner.scan(context, dir, config).await {
                    Ok(checks) => {
                        let check_count = checks.len();

                        // Filter by severity threshold
                        let filtered_checks: Vec<_> = checks
                            .into_iter()
                            .filter(|check| check.severity >= config.severity_threshold)
                            .collect();

                        let filtered_count = filtered_checks.len();
                        all_checks.extend(filtered_checks);

                        send_event(
                            context,
                            Event::OperationCompleted {
                                operation: format!(
                                    "{} found {} vulnerabilities ({} after filtering)",
                                    name, check_count, filtered_count
                                ),
                                success: true,
                            },
                        );
                    }
                    Err(e) => {
                        send_event(
                            context,
                            Event::BuildWarning {
                                package: context.name.clone(),
                                message: format!("Scanner {} failed: {}", name, e),
                            },
                        );

                        // Add a check for the scanner failure itself
                        all_checks.push(QaCheck::new(
                            QaCheckType::SecurityScanner,
                            name,
                            QaSeverity::Warning,
                            format!("Scanner failed to run: {}", e),
                        ));
                    }
                }
            }
        }

        Ok(all_checks)
    }
}

impl Default for ScannerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Common helper to run a scanner command
pub async fn run_scanner_command<S: ::std::hash::BuildHasher>(
    command: &str,
    args: &[String],
    working_dir: &Path,
    env: &HashMap<String, String, S>,
) -> Result<std::process::Output, Error> {
    let mut cmd = Command::new(command);
    cmd.args(args).current_dir(working_dir).env_clear();

    // Copy over essential environment variables
    for (key, value) in std::env::vars() {
        if key.starts_with("PATH") || key.starts_with("HOME") || key == "USER" {
            cmd.env(&key, &value);
        }
    }

    // Add custom environment variables
    for (key, value) in env {
        cmd.env(key, value);
    }

    cmd.output().await.map_err(|e| {
        BuildError::Failed {
            message: format!("Failed to run {}: {}", command, e),
        }
        .into()
    })
}

/// Common vulnerability severity mapping
pub fn map_cvss_to_severity(cvss_score: f64) -> QaSeverity {
    match cvss_score {
        score if score >= 9.0 => QaSeverity::Critical,
        score if score >= 7.0 => QaSeverity::Error,
        score if score >= 4.0 => QaSeverity::Warning,
        _ => QaSeverity::Info,
    }
}

/// Common vulnerability severity mapping by string
pub fn map_severity_string(severity: &str) -> QaSeverity {
    match severity.to_lowercase().as_str() {
        "critical" | "high" => QaSeverity::Critical,
        "medium" => QaSeverity::Error,
        "low" => QaSeverity::Warning,
        "info" | "informational" => QaSeverity::Info,
        _ => QaSeverity::Warning,
    }
}

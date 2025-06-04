//! Quality assurance pipeline for orchestrating all checks

use super::config::{QaConfig, ReportFormat};
use super::linters::LinterRegistry;
use super::policy::PolicyValidatorRegistry;
use super::reports::{QaReport, QaResult};
use super::scanners::ScannerRegistry;
use crate::events::send_event;
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;

/// Quality assurance pipeline
pub struct QaPipeline {
    config: QaConfig,
    linter_registry: Arc<LinterRegistry>,
    scanner_registry: Arc<ScannerRegistry>,
    policy_registry: Arc<PolicyValidatorRegistry>,
    semaphore: Arc<Semaphore>,
}

impl QaPipeline {
    /// Execute the QA pipeline
    pub async fn execute(
        &self,
        context: &BuildContext,
        environment: &BuildEnvironment,
    ) -> Result<QaReport, Error> {
        let start_time = Instant::now();
        let staging_dir = environment.staging_dir();

        send_event(
            context,
            Event::OperationStarted {
                operation: "Quality assurance pipeline starting".to_string(),
            },
        );

        // Run all checks in parallel
        let tasks = self.spawn_qa_tasks(context, staging_dir).await;
        let all_checks = self.collect_results(tasks).await?;

        // Create and finalize report
        let mut report = self.create_report(context, all_checks, start_time.elapsed().as_secs());
        self.add_metadata_to_report(&mut report);

        // Save report if configured
        if let Some(report_path) = &self.config.report_path {
            self.save_report(&report, report_path).await?;
        }

        self.send_completion_event(context, &report);
        Ok(report)
    }

    /// Spawn all QA tasks based on configuration
    async fn spawn_qa_tasks(
        &self,
        context: &BuildContext,
        staging_dir: &Path,
    ) -> Vec<tokio::task::JoinHandle<Result<QaResult, Error>>> {
        let mut tasks = Vec::new();

        if self.config.linters_enabled() {
            tasks.push(self.spawn_linter_task(context, staging_dir));
        }

        if self.config.scanners_enabled() {
            tasks.push(self.spawn_scanner_task(context, staging_dir));
        }

        if self.config.policy_validators_enabled() {
            tasks.push(self.spawn_policy_task(context, staging_dir));
        }

        tasks
    }

    /// Spawn linter task
    fn spawn_linter_task(
        &self,
        context: &BuildContext,
        staging_dir: &Path,
    ) -> tokio::task::JoinHandle<Result<QaResult, Error>> {
        let linter_registry = Arc::clone(&self.linter_registry);
        let linter_configs = self.config.linters.clone();
        let context_clone = context.clone();
        let staging_dir_clone = staging_dir.to_path_buf();
        let semaphore = Arc::clone(&self.semaphore);

        tokio::spawn(async move {
            let _permit = semaphore.acquire().await.map_err(|_| BuildError::Failed {
                message: "Failed to acquire semaphore".to_string(),
            })?;
            let start = Instant::now();
            let checks = linter_registry
                .lint_directory(&context_clone, &staging_dir_clone, &linter_configs)
                .await?;
            let duration_ms = start.elapsed().as_millis() as u64;
            Ok(QaResult {
                success: true,
                checks,
                duration_ms,
            })
        })
    }

    /// Spawn scanner task
    fn spawn_scanner_task(
        &self,
        context: &BuildContext,
        staging_dir: &Path,
    ) -> tokio::task::JoinHandle<Result<QaResult, Error>> {
        let scanner_registry = Arc::clone(&self.scanner_registry);
        let scanner_configs = self.config.scanners.clone();
        let context_clone = context.clone();
        let staging_dir_clone = staging_dir.to_path_buf();
        let semaphore = Arc::clone(&self.semaphore);

        tokio::spawn(async move {
            let _permit = semaphore.acquire().await.map_err(|_| BuildError::Failed {
                message: "Failed to acquire semaphore".to_string(),
            })?;
            let start = Instant::now();
            let checks = scanner_registry
                .scan_directory(&context_clone, &staging_dir_clone, &scanner_configs)
                .await?;
            let duration_ms = start.elapsed().as_millis() as u64;
            Ok(QaResult {
                success: true,
                checks,
                duration_ms,
            })
        })
    }

    /// Spawn policy validation task
    fn spawn_policy_task(
        &self,
        context: &BuildContext,
        staging_dir: &Path,
    ) -> tokio::task::JoinHandle<Result<QaResult, Error>> {
        let policy_registry = Arc::clone(&self.policy_registry);
        let policy_rules = self.config.policy_rules.clone();
        let context_clone = context.clone();
        let staging_dir_clone = staging_dir.to_path_buf();
        let semaphore = Arc::clone(&self.semaphore);

        tokio::spawn(async move {
            let _permit = semaphore.acquire().await.map_err(|_| BuildError::Failed {
                message: "Failed to acquire semaphore".to_string(),
            })?;
            let start = Instant::now();
            let checks = policy_registry
                .validate_all(&context_clone, &staging_dir_clone, &policy_rules)
                .await?;
            let duration_ms = start.elapsed().as_millis() as u64;
            Ok(QaResult {
                success: true,
                checks,
                duration_ms,
            })
        })
    }

    /// Collect results from all tasks
    async fn collect_results(
        &self,
        tasks: Vec<tokio::task::JoinHandle<Result<QaResult, Error>>>,
    ) -> Result<Vec<super::types::QaCheck>, Error> {
        let mut all_checks = Vec::new();

        for task in tasks {
            match task.await {
                Ok(Ok(result)) => {
                    all_checks.extend(result.checks);
                }
                Ok(Err(e)) => return Err(e),
                Err(e) => {
                    return Err(BuildError::Failed {
                        message: format!("QA task panicked: {}", e),
                    }
                    .into());
                }
            }
        }

        Ok(all_checks)
    }

    /// Create QA report
    fn create_report(
        &self,
        context: &BuildContext,
        all_checks: Vec<super::types::QaCheck>,
        duration_seconds: u64,
    ) -> QaReport {
        QaReport::new(
            &context.name,
            context.version.to_string(),
            all_checks,
            duration_seconds,
        )
    }

    /// Add metadata to report
    fn add_metadata_to_report(&self, report: &mut QaReport) {
        report.add_metadata("qa_level", format!("{:?}", self.config.level));
        report.add_metadata("linters_enabled", self.config.linters_enabled().to_string());
        report.add_metadata(
            "scanners_enabled",
            self.config.scanners_enabled().to_string(),
        );
        report.add_metadata(
            "policy_validators_enabled",
            self.config.policy_validators_enabled().to_string(),
        );
    }

    /// Send completion event
    fn send_completion_event(&self, context: &BuildContext, report: &QaReport) {
        send_event(
            context,
            Event::OperationCompleted {
                operation: format!(
                    "Quality assurance completed: {} checks, {} passed, {} failed",
                    report.total_checks(),
                    report.summary.passed,
                    report.summary.failed
                ),
                success: !report.has_errors(),
            },
        );
    }

    /// Save report to file
    async fn save_report(&self, report: &QaReport, path: &Path) -> Result<(), Error> {
        let content = match self.config.report_format {
            ReportFormat::Text => report.to_text(),
            ReportFormat::Json => {
                serde_json::to_string_pretty(report).map_err(|e| BuildError::Failed {
                    message: format!("Failed to serialize report: {}", e),
                })?
            }
            ReportFormat::Sarif => report.to_sarif().to_string(),
            ReportFormat::JUnit => report.to_junit(),
        };

        tokio::fs::write(path, content).await.map_err(|e| {
            BuildError::Failed {
                message: format!("Failed to write report: {}", e),
            }
            .into()
        })
    }
}

/// Builder for QA pipeline
pub struct QaPipelineBuilder {
    config: Option<QaConfig>,
    linters_enabled: bool,
    scanners_enabled: bool,
    policy_validators_enabled: bool,
}

impl QaPipelineBuilder {
    /// Create a new pipeline builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: None,
            linters_enabled: true,
            scanners_enabled: true,
            policy_validators_enabled: true,
        }
    }

    /// Set the QA configuration
    #[must_use]
    pub fn with_config(mut self, config: QaConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Enable/disable linters
    #[must_use]
    pub fn with_linters(mut self, enabled: bool) -> Self {
        self.linters_enabled = enabled;
        self
    }

    /// Enable/disable scanners
    #[must_use]
    pub fn with_scanners(mut self, enabled: bool) -> Self {
        self.scanners_enabled = enabled;
        self
    }

    /// Enable/disable policy validators
    #[must_use]
    pub fn with_policy_validators(mut self, enabled: bool) -> Self {
        self.policy_validators_enabled = enabled;
        self
    }

    /// Build the pipeline
    pub fn build(self) -> Result<QaPipeline, Error> {
        let mut config = self.config.unwrap_or_default();

        // Override enabled flags if specified
        if !self.linters_enabled {
            config
                .flags
                .remove(super::config::QaComponentFlags::LINTERS);
        }
        if !self.scanners_enabled {
            config
                .flags
                .remove(super::config::QaComponentFlags::SCANNERS);
        }
        if !self.policy_validators_enabled {
            config
                .flags
                .remove(super::config::QaComponentFlags::POLICY_VALIDATORS);
        }

        let semaphore = Arc::new(Semaphore::new(config.parallel_jobs));

        Ok(QaPipeline {
            config,
            linter_registry: Arc::new(LinterRegistry::new()),
            scanner_registry: Arc::new(ScannerRegistry::new()),
            policy_registry: Arc::new(PolicyValidatorRegistry::new()),
            semaphore,
        })
    }
}

impl Default for QaPipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

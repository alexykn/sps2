// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Quality assurance and validation system for sps2 packages
//!
//! This module provides comprehensive quality checks including:
//! - Code linting for multiple languages
//! - Security vulnerability scanning
//! - Policy validation and compliance
//! - License checking
//! - Binary size limits
//! - Custom policy rules

pub mod config;
pub mod linters;
pub mod pipeline;
pub mod policy;
pub mod reports;
pub mod scanners;
pub mod types;

use crate::events::send_event;
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;

pub use config::{QaConfig, QaLevel};
pub use pipeline::{QaPipeline, QaPipelineBuilder};
pub use reports::{QaReport, QaResult};
pub use types::{QaCheck, QaCheckType, QaSeverity};

/// Run comprehensive quality assurance checks on a built package
pub async fn run_qa_checks(
    context: &BuildContext,
    environment: &BuildEnvironment,
    config: &QaConfig,
) -> Result<QaReport, Error> {
    send_event(
        context,
        Event::OperationStarted {
            operation: "Running quality assurance checks".to_string(),
        },
    );

    // Build QA pipeline based on configuration
    let pipeline = QaPipelineBuilder::new()
        .with_config(config.clone())
        .with_linters(config.linters_enabled())
        .with_scanners(config.scanners_enabled())
        .with_policy_validators(config.policy_validators_enabled())
        .build()?;

    // Execute pipeline
    let report = pipeline.execute(context, environment).await?;

    // Check if we should fail on warnings
    if config.fail_on_warnings() && report.has_warnings() {
        return Err(BuildError::Failed {
            message: format!(
                "Quality assurance failed: {} warnings found",
                report.warning_count()
            ),
        }
        .into());
    }

    // Check if we have any errors
    if report.has_errors() {
        return Err(BuildError::Failed {
            message: format!(
                "Quality assurance failed: {} errors found",
                report.error_count()
            ),
        }
        .into());
    }

    send_event(
        context,
        Event::OperationCompleted {
            operation: format!(
                "Quality assurance completed: {} checks passed",
                report.total_checks()
            ),
            success: true,
        },
    );

    Ok(report)
}

/// Quick validation check for development builds
pub async fn quick_validate(
    context: &BuildContext,
    environment: &BuildEnvironment,
) -> Result<(), Error> {
    let config = QaConfig::minimal();
    let report = run_qa_checks(context, environment, &config).await?;

    if report.has_errors() {
        return Err(BuildError::Failed {
            message: "Quick validation failed".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Full validation check for release builds
pub async fn full_validate(
    context: &BuildContext,
    environment: &BuildEnvironment,
) -> Result<QaReport, Error> {
    let config = QaConfig::strict();
    run_qa_checks(context, environment, &config).await
}

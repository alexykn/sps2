use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;

use super::FailureContext;

/// Target package being evaluated by QA.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaTarget {
    pub package: String,
    pub version: Version,
}

/// QA level applied to the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QaLevel {
    Fast,
    Standard,
    Strict,
    Custom(String),
}

/// Status for an individual QA check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QaCheckStatus {
    Passed,
    Failed,
    Skipped,
}

/// Severity for QA findings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QaSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Individual finding emitted by a QA check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaFinding {
    pub message: String,
    pub severity: QaSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Summary emitted after a QA check completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaCheckSummary {
    pub name: String,
    pub category: String,
    pub status: QaCheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<QaFinding>,
}

/// QA events consumed by CLI/logging pipelines.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QaEvent {
    PipelineStarted {
        target: QaTarget,
        level: QaLevel,
    },
    PipelineCompleted {
        target: QaTarget,
        total_checks: usize,
        passed: usize,
        failed: usize,
        duration_ms: u64,
    },
    PipelineFailed {
        target: QaTarget,
        failure: FailureContext,
    },
    CheckEvaluated {
        target: QaTarget,
        summary: QaCheckSummary,
    },
}

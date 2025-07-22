use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Quality assurance events for artifact validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum QaEvent {
    /// QA pipeline started
    PipelineStarted {
        package: String,
        version: String,
        qa_level: String,
    },

    /// QA pipeline completed
    PipelineCompleted {
        package: String,
        version: String,
        total_checks: usize,
        passed: usize,
        failed: usize,
        duration_seconds: u64,
    },

    /// QA check started
    CheckStarted {
        check_type: String,
        check_name: String,
    },

    /// QA check completed
    CheckCompleted {
        check_type: String,
        check_name: String,
        findings_count: usize,
        severity_counts: HashMap<String, usize>,
    },

    /// QA check failed
    CheckFailed {
        check_type: String,
        check_name: String,
        error: String,
    },

    /// QA finding reported
    FindingReported {
        check_type: String,
        severity: String,
        message: String,
        file_path: Option<String>,
        line: Option<usize>,
    },

    /// QA report generated
    ReportGenerated {
        format: String,
        path: Option<String>,
    },

    /// QA validation phase started
    ValidationPhaseStarted {
        phase: String,
        validators: Vec<String>,
    },

    /// QA validation phase completed
    ValidationPhaseCompleted {
        phase: String,
        findings_count: usize,
    },

    /// QA patching phase started
    PatchingPhaseStarted { patchers: Vec<String> },

    /// QA patching phase completed
    PatchingPhaseCompleted { patches_applied: usize },
}

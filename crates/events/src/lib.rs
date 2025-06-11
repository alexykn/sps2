#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Event system for async communication in sps2
//!
//! This crate provides the event types and channel aliases used for
//! communication between crates. All output goes through events - no
//! direct logging or printing is allowed outside the CLI.
//!
//! ## Progress Tracking
//!
//! This crate includes sophisticated progress tracking algorithms with:
//! - Speed calculation with smoothing and outlier detection
//! - Accurate ETA calculations with adaptive windows
//! - Phase-aware progress for multi-stage operations
//! - Memory-efficient data structures (<1KB per tracker)

pub mod progress;

pub use progress::*;

use serde::{Deserialize, Serialize};
use sps2_types::{StateId, Version};
use std::collections::HashMap;
use std::time::Duration;

/// Type alias for event sender
pub type EventSender = tokio::sync::mpsc::UnboundedSender<Event>;

/// Type alias for event receiver
pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<Event>;

/// Create a new event channel
#[must_use]
pub fn channel() -> (EventSender, EventReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Core event enum for all async communication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    // Download events
    DownloadStarted {
        url: String,
        size: Option<u64>,
    },
    DownloadProgress {
        url: String,
        bytes_downloaded: u64,
        total_bytes: u64,
    },
    DownloadCompleted {
        url: String,
        size: u64,
    },
    DownloadFailed {
        url: String,
        error: String,
    },
    DownloadResuming {
        url: String,
        offset: u64,
        total_size: Option<u64>,
    },
    DownloadInterrupted {
        url: String,
        bytes_downloaded: u64,
        error: String,
    },

    // Build events
    BuildStarting {
        package: String,
        version: Version,
    },
    BuildStepStarted {
        package: String,
        step: String,
    },
    BuildStepOutput {
        package: String,
        line: String,
    },
    BuildStepCompleted {
        package: String,
        step: String,
    },
    BuildCompleted {
        package: String,
        version: Version,
        path: std::path::PathBuf,
    },
    BuildFailed {
        package: String,
        version: Version,
        error: String,
    },
    BuildCommand {
        package: String,
        command: String,
    },
    BuildCleaned {
        package: String,
    },
    BuildRetrying {
        package: String,
        attempt: usize,
        reason: String,
    },
    BuildWarning {
        package: String,
        message: String,
    },
    BuildCheckpointCreated {
        package: String,
        checkpoint_id: String,
        stage: String,
    },
    BuildCheckpointRestored {
        package: String,
        checkpoint_id: String,
        stage: String,
    },
    BuildCacheHit {
        cache_key: String,
        artifacts_count: usize,
    },
    BuildCacheMiss {
        cache_key: String,
        reason: String,
    },
    BuildCacheUpdated {
        cache_key: String,
        artifacts_count: usize,
    },
    BuildCacheCleaned {
        removed_items: usize,
        freed_bytes: u64,
    },

    // State management
    StateTransition {
        from: StateId,
        to: StateId,
        operation: String,
    },
    StateRollback {
        from: StateId,
        to: StateId,
    },

    // Package operations
    PackageInstalling {
        name: String,
        version: Version,
    },
    PackageRemoving {
        name: String,
        version: Version,
    },
    PackageRemoved {
        name: String,
        version: Version,
    },
    PackageBuilding {
        name: String,
        version: Version,
    },

    // Resolution
    ResolvingDependencies {
        package: String,
    },
    DependencyResolved {
        package: String,
        version: Version,
        count: usize,
    },

    // Command completion
    ListStarting,
    ListCompleted {
        count: usize,
    },
    SearchStarting {
        query: String,
    },
    SearchCompleted {
        query: String,
        count: usize,
    },

    // Repository operations
    RepoSyncStarting,
    RepoSyncStarted {
        url: String,
    },
    RepoSyncProgress {
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
    },
    RepoSyncCompleted {
        packages_updated: usize,
        duration_ms: u64,
    },

    // Errors and warnings
    Warning {
        message: String,
        context: Option<String>,
    },
    Error {
        message: String,
        details: Option<String>,
    },

    // Debug logging (when --debug enabled)
    DebugLog {
        message: String,
        context: HashMap<String, String>,
    },

    // General progress
    OperationStarted {
        operation: String,
    },
    OperationCompleted {
        operation: String,
        success: bool,
    },

    // Quality Assurance events
    QaCheckStarted {
        check_type: String,
        check_name: String,
    },
    QaCheckCompleted {
        check_type: String,
        check_name: String,
        findings_count: usize,
        severity_counts: HashMap<String, usize>,
    },
    QaCheckFailed {
        check_type: String,
        check_name: String,
        error: String,
    },
    QaPipelineStarted {
        package: String,
        version: String,
        qa_level: String,
    },
    QaPipelineCompleted {
        package: String,
        version: String,
        total_checks: usize,
        passed: usize,
        failed: usize,
        duration_seconds: u64,
    },
    QaFindingReported {
        check_type: String,
        severity: String,
        message: String,
        file_path: Option<String>,
        line: Option<usize>,
    },
    QaReportGenerated {
        format: String,
        path: Option<String>,
    },
    OperationFailed {
        operation: String,
        error: String,
    },

    // Index operations
    IndexUpdateStarting {
        url: String,
    },
    IndexUpdateCompleted {
        packages_added: usize,
        packages_updated: usize,
    },

    // Cleanup operations
    CleanupStarting,
    CleanupProgress {
        items_processed: usize,
        total_items: usize,
    },
    CleanupCompleted {
        states_removed: usize,
        packages_removed: usize,
        duration_ms: u64,
    },

    // Health check
    HealthCheckStarting,
    HealthCheckStarted,
    HealthCheckProgress {
        component: String,
        status: HealthStatus,
        message: Option<String>,
    },
    HealthCheckCompleted {
        healthy: bool,
        issues: Vec<String>,
    },

    // Advanced Progress Tracking
    ProgressStarted {
        id: String,
        operation: String,
        total: Option<u64>,
        phases: Vec<ProgressPhase>,
    },
    ProgressUpdated {
        id: String,
        current: u64,
        total: Option<u64>,
        phase: Option<usize>,
        speed: Option<f64>,
        eta: Option<Duration>,
    },
    ProgressPhaseChanged {
        id: String,
        phase: usize,
        phase_name: String,
    },
    ProgressCompleted {
        id: String,
        duration: Duration,
    },
    ProgressFailed {
        id: String,
        error: String,
    },

    // Audit operations
    AuditStarting {
        package_count: usize,
    },
    AuditPackageCompleted {
        package: String,
        vulnerabilities_found: usize,
    },
    AuditCompleted {
        packages_scanned: usize,
        vulnerabilities_found: usize,
        critical_count: usize,
    },

    // Vulnerability database operations
    VulnDbUpdateStarting,
    VulnDbSourceUpdateStarting {
        source: String,
    },
    VulnDbSourceUpdateProgress {
        source: String,
        processed: usize,
        total: Option<usize>,
    },
    VulnDbSourceUpdateCompleted {
        source: String,
        vulnerabilities_added: usize,
        duration_ms: u64,
    },
    VulnDbSourceUpdateFailed {
        source: String,
        error: String,
    },
    VulnDbUpdateCompleted {
        total_vulnerabilities: usize,
        sources_updated: usize,
        duration_ms: u64,
    },

    // Install operations
    StateCreating {
        state_id: uuid::Uuid,
    },
    InstallStarting {
        packages: Vec<String>,
    },
    InstallCompleted {
        packages: Vec<String>,
        state_id: uuid::Uuid,
    },

    // Package download events
    PackageDownloadStarted {
        name: String,
        version: Version,
        url: String,
    },
    PackageDownloaded {
        name: String,
        version: Version,
    },
    PackageInstalled {
        name: String,
        version: Version,
        path: String,
    },
    PackageSignatureDownloaded {
        name: String,
        version: Version,
        verified: bool,
    },

    // Dependency resolution
    DependencyResolving {
        package: String,
        count: usize,
    },

    // Uninstall operations
    UninstallStarting {
        packages: Vec<String>,
    },
    UninstallCompleted {
        packages: Vec<String>,
        state_id: uuid::Uuid,
    },

    // Update operations
    UpdateStarting {
        packages: Vec<String>,
    },
    UpdateCompleted {
        packages: Vec<String>,
        state_id: uuid::Uuid,
    },

    // Upgrade operations
    UpgradeStarting {
        packages: Vec<String>,
    },
    UpgradeCompleted {
        packages: Vec<String>,
        state_id: uuid::Uuid,
    },

    // Rollback operations
    RollbackStarting {
        target_state: uuid::Uuid,
    },
    RollbackCompleted {
        target_state: uuid::Uuid,
        duration_ms: u64,
    },

    // Self-update operations
    SelfUpdateStarting,
    SelfUpdateCheckingVersion {
        current_version: String,
    },
    SelfUpdateVersionAvailable {
        current_version: String,
        latest_version: String,
    },
    SelfUpdateAlreadyLatest {
        version: String,
    },
    SelfUpdateDownloading {
        version: String,
        url: String,
    },
    SelfUpdateVerifying {
        version: String,
    },
    SelfUpdateInstalling {
        version: String,
    },
    SelfUpdateCompleted {
        old_version: String,
        new_version: String,
        duration_ms: u64,
    },

    // Python virtual environment operations
    PythonVenvCreating {
        package: String,
        version: Version,
        venv_path: String,
    },
    PythonVenvCreated {
        package: String,
        version: Version,
        venv_path: String,
    },
    PythonWheelInstalling {
        package: String,
        version: Version,
        wheel_file: String,
    },
    PythonWheelInstalled {
        package: String,
        version: Version,
    },
    PythonWrapperCreating {
        package: String,
        executable: String,
        wrapper_path: String,
    },
    PythonWrapperCreated {
        package: String,
        executable: String,
        wrapper_path: String,
    },
    PythonVenvCloning {
        package: String,
        version: Version,
        from_path: String,
        to_path: String,
    },
    PythonVenvCloned {
        package: String,
        version: Version,
        from_path: String,
        to_path: String,
    },
    PythonVenvRemoving {
        package: String,
        version: Version,
        venv_path: String,
    },
    PythonVenvRemoved {
        package: String,
        version: Version,
        venv_path: String,
    },
}

/// Health status for components
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Warning,
    Error,
}

impl Event {
    /// Create a warning event
    pub fn warning(message: impl Into<String>) -> Self {
        Self::Warning {
            message: message.into(),
            context: None,
        }
    }

    /// Create an error event
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            details: None,
        }
    }

    /// Create a debug log event
    pub fn debug(message: impl Into<String>) -> Self {
        Self::DebugLog {
            message: message.into(),
            context: HashMap::new(),
        }
    }
}

/// Helper to send events with error handling
pub trait EventSenderExt {
    /// Send an event, ignoring send errors (receiver dropped)
    fn emit(&self, event: Event);
}

impl EventSenderExt for EventSender {
    fn emit(&self, event: Event) {
        // Ignore send errors - if receiver is dropped, we just continue
        let _ = self.send(event);
    }
}

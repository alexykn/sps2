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

// Import for the EventEmitter implementation
use tokio::sync::mpsc::UnboundedSender;

/// Type alias for event receiver
pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<Event>;

/// Create a new event channel
#[must_use]
pub fn channel() -> (EventSender, EventReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Parameters for creating a guard discrepancy found event
#[derive(Debug, Clone)]
pub struct GuardDiscrepancyParams {
    pub operation_id: String,
    pub discrepancy_type: String,
    pub severity: String,
    pub file_path: String,
    pub package: Option<String>,
    pub package_version: Option<String>,
    pub user_message: String,
    pub technical_details: String,
    pub auto_heal_available: bool,
    pub requires_confirmation: bool,
    pub estimated_fix_time_seconds: Option<u64>,
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

    // Two-Phase Commit events
    TwoPhaseCommitStarting {
        state_id: uuid::Uuid,
        parent_state_id: uuid::Uuid,
        operation: String,
    },
    TwoPhaseCommitPhaseOneStarting {
        state_id: uuid::Uuid,
        operation: String,
    },
    TwoPhaseCommitPhaseOneCompleted {
        state_id: uuid::Uuid,
        operation: String,
    },
    TwoPhaseCommitPhaseTwoStarting {
        state_id: uuid::Uuid,
        operation: String,
    },
    TwoPhaseCommitPhaseTwoCompleted {
        state_id: uuid::Uuid,
        operation: String,
    },
    TwoPhaseCommitCompleted {
        state_id: uuid::Uuid,
        parent_state_id: uuid::Uuid,
        operation: String,
    },
    TwoPhaseCommitFailed {
        state_id: uuid::Uuid,
        operation: String,
        error: String,
        phase: String,
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

    // Guard verification events
    GuardVerificationStarted {
        operation_id: String,
        scope: String,
        level: String,
        packages_count: usize,
        files_count: Option<usize>,
    },
    GuardVerificationProgress {
        operation_id: String,
        verified_packages: usize,
        total_packages: usize,
        verified_files: usize,
        total_files: usize,
        current_package: Option<String>,
        cache_hit_rate: Option<f64>,
    },
    GuardDiscrepancyFound {
        operation_id: String,
        discrepancy_type: String,
        severity: String,
        file_path: String,
        package: Option<String>,
        package_version: Option<String>,
        user_message: String,
        technical_details: String,
        auto_heal_available: bool,
        requires_confirmation: bool,
        estimated_fix_time_seconds: Option<u64>,
    },
    GuardVerificationCompleted {
        operation_id: String,
        total_discrepancies: usize,
        by_severity: HashMap<String, usize>,
        duration_ms: u64,
        cache_hit_rate: f64,
        coverage_percent: f64,
        scope_description: String,
    },
    GuardVerificationFailed {
        operation_id: String,
        error: String,
        packages_verified: usize,
        files_verified: usize,
        duration_ms: u64,
    },

    // Guard healing events
    GuardHealingStarted {
        operation_id: String,
        discrepancies_count: usize,
        auto_heal_count: usize,
        confirmation_required_count: usize,
        manual_intervention_count: usize,
    },
    GuardHealingProgress {
        operation_id: String,
        completed: usize,
        total: usize,
        current_operation: String,
        current_file: Option<String>,
    },
    GuardHealingResult {
        operation_id: String,
        discrepancy_type: String,
        file_path: String,
        success: bool,
        healing_action: String,
        error: Option<String>,
        duration_ms: u64,
    },
    GuardHealingCompleted {
        operation_id: String,
        healed_count: usize,
        failed_count: usize,
        skipped_count: usize,
        duration_ms: u64,
    },
    GuardHealingFailed {
        operation_id: String,
        error: String,
        completed_healing: usize,
        failed_healing: usize,
        duration_ms: u64,
    },

    // Guard cache events
    GuardCacheWarming {
        operation_id: String,
        operation_type: String,
        cache_entries_loading: usize,
    },
    GuardCacheWarmingCompleted {
        operation_id: String,
        cache_entries_loaded: usize,
        cache_hit_rate_improvement: f64,
        duration_ms: u64,
    },
    GuardCacheInvalidated {
        operation_id: String,
        operation_type: String,
        invalidated_entries: usize,
        reason: String,
    },

    // Guard error summary events
    GuardErrorSummary {
        operation_id: String,
        total_errors: usize,
        recoverable_errors: usize,
        manual_intervention_required: usize,
        overall_severity: String,
        user_friendly_summary: String,
        recommended_actions: Vec<String>,
    },

    // Guard configuration events
    GuardConfigurationValidated {
        approach: String, // "top-level" or "nested"
        enabled: bool,
        verification_level: String,
        auto_heal: bool,
        validation_warnings: Vec<String>,
    },
    GuardConfigurationError {
        field: String,
        error: String,
        suggested_fix: Option<String>,
        current_value: Option<String>,
    },

    // Guard recovery events
    GuardRecoveryAttempt {
        operation_id: String,
        error_category: String,
        recovery_strategy: String,
        attempt_number: usize,
        max_attempts: usize,
    },
    GuardRecoverySuccess {
        operation_id: String,
        error_category: String,
        recovery_strategy: String,
        attempt_number: usize,
        recovery_duration_ms: u64,
    },
    GuardRecoveryFailed {
        operation_id: String,
        error_category: String,
        recovery_strategy: String,
        attempts_made: usize,
        final_error: String,
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

    /// Create a guard discrepancy found event
    #[must_use]
    pub fn guard_discrepancy_found(params: GuardDiscrepancyParams) -> Self {
        Self::GuardDiscrepancyFound {
            operation_id: params.operation_id,
            discrepancy_type: params.discrepancy_type,
            severity: params.severity,
            file_path: params.file_path,
            package: params.package,
            package_version: params.package_version,
            user_message: params.user_message,
            technical_details: params.technical_details,
            auto_heal_available: params.auto_heal_available,
            requires_confirmation: params.requires_confirmation,
            estimated_fix_time_seconds: params.estimated_fix_time_seconds,
        }
    }

    /// Create a guard error summary event
    pub fn guard_error_summary(
        operation_id: impl Into<String>,
        total_errors: usize,
        recoverable_errors: usize,
        manual_intervention_required: usize,
        overall_severity: impl Into<String>,
        user_friendly_summary: impl Into<String>,
        recommended_actions: Vec<String>,
    ) -> Self {
        Self::GuardErrorSummary {
            operation_id: operation_id.into(),
            total_errors,
            recoverable_errors,
            manual_intervention_required,
            overall_severity: overall_severity.into(),
            user_friendly_summary: user_friendly_summary.into(),
            recommended_actions,
        }
    }

    /// Create a guard verification started event
    pub fn guard_verification_started(
        operation_id: impl Into<String>,
        scope: impl Into<String>,
        level: impl Into<String>,
        packages_count: usize,
        files_count: Option<usize>,
    ) -> Self {
        Self::GuardVerificationStarted {
            operation_id: operation_id.into(),
            scope: scope.into(),
            level: level.into(),
            packages_count,
            files_count,
        }
    }

    /// Create a guard verification completed event
    pub fn guard_verification_completed(
        operation_id: impl Into<String>,
        total_discrepancies: usize,
        by_severity: HashMap<String, usize>,
        duration_ms: u64,
        cache_hit_rate: f64,
        coverage_percent: f64,
        scope_description: impl Into<String>,
    ) -> Self {
        Self::GuardVerificationCompleted {
            operation_id: operation_id.into(),
            total_discrepancies,
            by_severity,
            duration_ms,
            cache_hit_rate,
            coverage_percent,
            scope_description: scope_description.into(),
        }
    }

    /// Create a guard healing result event
    pub fn guard_healing_result(
        operation_id: impl Into<String>,
        discrepancy_type: impl Into<String>,
        file_path: impl Into<String>,
        success: bool,
        healing_action: impl Into<String>,
        error: Option<String>,
        duration_ms: u64,
    ) -> Self {
        Self::GuardHealingResult {
            operation_id: operation_id.into(),
            discrepancy_type: discrepancy_type.into(),
            file_path: file_path.into(),
            success,
            healing_action: healing_action.into(),
            error,
            duration_ms,
        }
    }

    /// Create a guard configuration error event
    pub fn guard_configuration_error(
        field: impl Into<String>,
        error: impl Into<String>,
        suggested_fix: Option<String>,
        current_value: Option<String>,
    ) -> Self {
        Self::GuardConfigurationError {
            field: field.into(),
            error: error.into(),
            suggested_fix,
            current_value,
        }
    }
}

/// Helper to send events with error handling
pub trait EventSenderExt {
    /// Send an event, ignoring send errors (receiver dropped)
    fn emit(&self, event: Event);

    /// Send a debug log event
    fn emit_debug(&self, message: impl Into<String>) {
        self.emit(Event::debug(message));
    }

    /// Send a warning event
    fn emit_warning(&self, message: impl Into<String>) {
        self.emit(Event::warning(message));
    }

    /// Send an error event
    fn emit_error(&self, message: impl Into<String>) {
        self.emit(Event::error(message));
    }

    /// Send an operation started event
    fn emit_operation_started(&self, operation: impl Into<String>) {
        self.emit(Event::OperationStarted {
            operation: operation.into(),
        });
    }

    /// Send an operation completed event
    fn emit_operation_completed(&self, operation: impl Into<String>, success: bool) {
        self.emit(Event::OperationCompleted {
            operation: operation.into(),
            success,
        });
    }

    /// Send an operation failed event
    fn emit_operation_failed(&self, operation: impl Into<String>, error: impl Into<String>) {
        self.emit(Event::OperationFailed {
            operation: operation.into(),
            error: error.into(),
        });
    }
}

impl EventSenderExt for EventSender {
    fn emit(&self, event: Event) {
        // Ignore send errors - if receiver is dropped, we just continue
        let _ = self.send(event);
    }
}

/// Global event emitter for convenient access across crates
pub struct GlobalEventEmitter {
    sender: Option<EventSender>,
}

impl Default for GlobalEventEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalEventEmitter {
    /// Create a new global event emitter with no sender (events will be ignored)
    #[must_use]
    pub const fn new() -> Self {
        Self { sender: None }
    }

    /// Initialize the global event emitter with a sender
    pub fn init(&mut self, sender: EventSender) {
        self.sender = Some(sender);
    }

    /// Check if the global event emitter is initialized
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.sender.is_some()
    }

    /// Get the current event sender, if available
    #[must_use]
    pub fn sender(&self) -> Option<&EventSender> {
        self.sender.as_ref()
    }
}

impl EventSenderExt for GlobalEventEmitter {
    fn emit(&self, event: Event) {
        if let Some(sender) = &self.sender {
            sender.emit(event);
        }
    }
}

/// Global event emitter instance
static GLOBAL_EVENT_EMITTER: std::sync::RwLock<GlobalEventEmitter> =
    std::sync::RwLock::new(GlobalEventEmitter::new());

/// Initialize the global event emitter
pub fn init_global_event_emitter(sender: EventSender) {
    if let Ok(mut emitter) = GLOBAL_EVENT_EMITTER.write() {
        emitter.init(sender);
    }
}

/// Get access to the global event emitter
#[must_use]
pub fn global_event_emitter() -> impl EventSenderExt {
    struct GlobalEmitter;

    impl EventSenderExt for GlobalEmitter {
        fn emit(&self, event: Event) {
            if let Ok(emitter) = GLOBAL_EVENT_EMITTER.read() {
                emitter.emit(event);
            }
        }
    }

    GlobalEmitter
}

/// Trait for types that can emit events
pub trait EventEmitter {
    /// Get the event sender for this emitter
    fn event_sender(&self) -> Option<&EventSender>;

    /// Emit an event through this emitter
    fn emit_event(&self, event: Event) {
        if let Some(sender) = self.event_sender() {
            sender.emit(event);
        }
    }

    /// Emit a debug log event
    fn emit_debug(&self, message: impl Into<String>) {
        self.emit_event(Event::debug(message));
    }

    /// Emit a warning event
    fn emit_warning(&self, message: impl Into<String>) {
        self.emit_event(Event::warning(message));
    }

    /// Emit an error event
    fn emit_error(&self, message: impl Into<String>) {
        self.emit_event(Event::error(message));
    }

    /// Emit an operation started event
    fn emit_operation_started(&self, operation: impl Into<String>) {
        self.emit_event(Event::OperationStarted {
            operation: operation.into(),
        });
    }

    /// Emit an operation completed event
    fn emit_operation_completed(&self, operation: impl Into<String>, success: bool) {
        self.emit_event(Event::OperationCompleted {
            operation: operation.into(),
            success,
        });
    }

    /// Emit an operation failed event
    fn emit_operation_failed(&self, operation: impl Into<String>, error: impl Into<String>) {
        self.emit_event(Event::OperationFailed {
            operation: operation.into(),
            error: error.into(),
        });
    }
}

/// Implementation of `EventEmitter` for `UnboundedSender<Event>`
/// This allows `UnboundedSender` to be used directly where `EventEmitter` is expected
impl EventEmitter for UnboundedSender<Event> {
    fn event_sender(&self) -> Option<&EventSender> {
        Some(self)
    }
}

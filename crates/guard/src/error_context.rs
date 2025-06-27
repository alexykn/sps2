//! Error context and metadata collection utilities for guard operations

use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

use sps2_errors::{DiscrepancySeverity, GuardError, GuardErrorSummary};
use sps2_events::{Event, EventSender, EventSenderExt};

use crate::types::{Discrepancy, OperationType, VerificationResult};

/// Context collector for guard operations to provide rich error reporting
#[derive(Debug)]
pub struct GuardErrorContext {
    /// Unique operation ID for tracking events
    operation_id: String,
    /// Type of operation being performed
    operation_type: OperationType,
    /// Start time of the operation
    start_time: Instant,
    /// Event sender for reporting
    event_sender: EventSender,
    /// Collected errors during operation
    errors: Vec<GuardError>,
    /// Collected discrepancies
    discrepancies: Vec<Discrepancy>,
    /// Operation metadata
    metadata: HashMap<String, String>,
    /// Verbosity level for reporting
    verbosity_level: VerbosityLevel,
}

/// Verbosity levels for error reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbosityLevel {
    /// Minimal output - only critical issues
    Minimal,
    /// Standard output - important information
    Standard,
    /// Detailed output - comprehensive information
    Detailed,
    /// Debug output - all available information
    Debug,
}

impl Default for VerbosityLevel {
    fn default() -> Self {
        Self::Standard
    }
}

impl VerbosityLevel {
    /// Check if this verbosity level should include detailed context
    #[must_use]
    pub fn include_detailed_context(self) -> bool {
        matches!(self, Self::Detailed | Self::Debug)
    }

    /// Check if this verbosity level should include technical details
    #[must_use]
    pub fn include_technical_details(self) -> bool {
        matches!(self, Self::Debug)
    }

    /// Check if severity level should be reported at this verbosity
    #[must_use]
    pub fn should_report_severity(self, severity: DiscrepancySeverity) -> bool {
        match self {
            Self::Minimal => matches!(severity, DiscrepancySeverity::Critical),
            Self::Standard => !matches!(severity, DiscrepancySeverity::Low),
            Self::Detailed | Self::Debug => true,
        }
    }
}

impl GuardErrorContext {
    /// Create a new error context for a guard operation
    #[must_use]
    pub fn new(
        operation_type: OperationType,
        event_sender: EventSender,
        verbosity_level: VerbosityLevel,
    ) -> Self {
        let operation_id = Uuid::new_v4().to_string();
        
        Self {
            operation_id,
            operation_type,
            start_time: Instant::now(),
            event_sender,
            errors: Vec::new(),
            discrepancies: Vec::new(),
            metadata: HashMap::new(),
            verbosity_level,
        }
    }

    /// Get the operation ID
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Get the verbosity level
    #[must_use]
    pub fn verbosity_level(&self) -> VerbosityLevel {
        self.verbosity_level
    }

    /// Add metadata to the context
    pub fn add_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }

    /// Add multiple metadata entries
    pub fn add_metadata_map(&mut self, metadata: HashMap<String, String>) {
        self.metadata.extend(metadata);
    }

    /// Record a guard error
    pub fn record_error(&mut self, error: GuardError) {
        // Emit error event if verbosity allows it
        if self.verbosity_level.should_report_severity(error.severity()) {
            let context = error.user_context();
            
            self.event_sender.emit(Event::guard_discrepancy_found(
                &self.operation_id,
                format!("{:?}", error),
                format!("{:?}", error.severity()),
                "", // No specific file path for general errors
                None,
                None,
                context.user_message.clone(),
                context.technical_details.clone(),
                error.is_recoverable(),
                context.requires_user_action(),
                context.estimated_fix_time.map(|d| d.as_secs()),
            ));
        }

        self.errors.push(error);
    }

    /// Record a verification discrepancy
    pub fn record_discrepancy(&mut self, discrepancy: Discrepancy) {
        // Emit discrepancy event if verbosity allows it
        if self.verbosity_level.should_report_severity(discrepancy.severity()) {
            let context = discrepancy.user_context();
            
            self.event_sender.emit(Event::guard_discrepancy_found(
                &self.operation_id,
                discrepancy.short_description(),
                format!("{:?}", discrepancy.severity()),
                discrepancy.file_path(),
                discrepancy.package_name().map(ToString::to_string),
                discrepancy.package_version().map(ToString::to_string),
                context.user_message.clone(),
                if self.verbosity_level.include_technical_details() {
                    context.technical_details.clone()
                } else {
                    discrepancy.short_description()
                },
                discrepancy.can_auto_heal(),
                discrepancy.requires_confirmation(),
                context.estimated_fix_time.map(|d| d.as_secs()),
            ));
        }

        self.discrepancies.push(discrepancy);
    }

    /// Record multiple discrepancies
    pub fn record_discrepancies(&mut self, discrepancies: Vec<Discrepancy>) {
        for discrepancy in discrepancies {
            self.record_discrepancy(discrepancy);
        }
    }

    /// Record discrepancies from a verification result
    pub fn record_verification_result(&mut self, result: &VerificationResult) {
        self.record_discrepancies(result.discrepancies.clone());
        
        // Add metadata about the verification
        self.add_metadata("verification_duration_ms", result.duration_ms.to_string());
        self.add_metadata("verification_state_id", result.state_id.to_string());
        self.add_metadata("verification_valid", result.is_valid.to_string());
        
        if let Some(coverage) = &result.coverage {
            self.add_metadata("coverage_packages", format!("{}/{}", coverage.verified_packages, coverage.total_packages));
            self.add_metadata("coverage_files", format!("{}/{}", coverage.verified_files, coverage.total_files));
            self.add_metadata("coverage_package_percent", format!("{:.1}%", coverage.package_coverage_percent));
            self.add_metadata("coverage_file_percent", format!("{:.1}%", coverage.file_coverage_percent));
        }
    }

    /// Create and emit an error summary based on collected errors and discrepancies
    pub fn emit_error_summary(&self) {
        let all_errors: Vec<GuardError> = self.discrepancies
            .iter()
            .map(|d| {
                let context = d.user_context();
                GuardError::VerificationFailed {
                    operation: format!("{:?}", self.operation_type),
                    details: context.user_message.clone(),
                    discrepancies_count: 1,
                    state_id: "unknown".to_string(),
                    duration_ms: self.start_time.elapsed().as_millis() as u64,
                }
            })
            .chain(self.errors.iter().cloned())
            .collect();

        if all_errors.is_empty() {
            // No errors to report
            if self.verbosity_level.include_detailed_context() {
                self.event_sender.emit(Event::guard_error_summary(
                    &self.operation_id,
                    0,
                    0,
                    0,
                    "Low",
                    "No issues found",
                    vec!["No action required".to_string()],
                ));
            }
            return;
        }

        let summary = GuardErrorSummary::new(all_errors);
        
        // Create verbosity-appropriate recommended actions
        let mut recommended_actions = summary.recommended_actions.clone();
        
        if self.verbosity_level.include_detailed_context() {
            // Add detailed recommendations
            if !summary.recoverable_errors.is_empty() {
                recommended_actions.push(format!("Use 'sps2 verify --heal' to automatically fix {} issue(s)", summary.recoverable_errors.len()));
            }
            
            if !summary.manual_intervention_required.is_empty() {
                recommended_actions.push("Check individual error details for manual resolution steps".to_string());
            }
        }

        self.event_sender.emit(Event::guard_error_summary(
            &self.operation_id,
            summary.total_errors,
            summary.recoverable_errors.len(),
            summary.manual_intervention_required.len(),
            format!("{:?}", summary.overall_severity),
            summary.summary_message(),
            recommended_actions,
        ));
    }

    /// Emit operation start event
    pub fn emit_operation_start(&self, scope: &str, level: &str, packages_count: usize, files_count: Option<usize>) {
        self.event_sender.emit(Event::guard_verification_started(
            &self.operation_id,
            scope,
            level,
            packages_count,
            files_count,
        ));
    }

    /// Emit operation completion event
    pub fn emit_operation_completed(&self, cache_hit_rate: f64, coverage_percent: f64, scope_description: &str) {
        let by_severity = self.get_severity_counts();
        
        self.event_sender.emit(Event::guard_verification_completed(
            &self.operation_id,
            self.discrepancies.len(),
            by_severity,
            self.start_time.elapsed().as_millis() as u64,
            cache_hit_rate,
            coverage_percent,
            scope_description,
        ));
    }

    /// Emit healing result event
    pub fn emit_healing_result(
        &self,
        discrepancy_type: &str,
        file_path: &str,
        success: bool,
        healing_action: &str,
        error: Option<String>,
    ) {
        let start_time = Instant::now();
        
        self.event_sender.emit(Event::guard_healing_result(
            &self.operation_id,
            discrepancy_type,
            file_path,
            success,
            healing_action,
            error,
            start_time.elapsed().as_millis() as u64,
        ));
    }

    /// Get severity counts for collected discrepancies
    #[must_use]
    pub fn get_severity_counts(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        
        for discrepancy in &self.discrepancies {
            let severity = format!("{:?}", discrepancy.severity());
            *counts.entry(severity).or_insert(0) += 1;
        }
        
        for error in &self.errors {
            let severity = format!("{:?}", error.severity());
            *counts.entry(severity).or_insert(0) += 1;
        }
        
        counts
    }

    /// Get summary statistics
    #[must_use]
    pub fn get_summary_stats(&self) -> ContextSummaryStats {
        let total_issues = self.discrepancies.len() + self.errors.len();
        let recoverable_count = self.discrepancies.iter().filter(|d| d.can_auto_heal()).count() 
            + self.errors.iter().filter(|e| e.is_recoverable()).count();
        let confirmation_required = self.discrepancies.iter().filter(|d| d.requires_confirmation()).count();
        
        let overall_severity = self.discrepancies
            .iter()
            .map(|d| d.severity())
            .chain(self.errors.iter().map(|e| e.severity()))
            .max()
            .unwrap_or(DiscrepancySeverity::Low);

        ContextSummaryStats {
            total_issues,
            recoverable_count,
            confirmation_required,
            manual_intervention_required: total_issues - recoverable_count - confirmation_required,
            overall_severity,
            operation_duration: self.start_time.elapsed(),
        }
    }

    /// Check if operation has any critical issues
    #[must_use]
    pub fn has_critical_issues(&self) -> bool {
        self.discrepancies.iter().any(|d| matches!(d.severity(), DiscrepancySeverity::Critical))
            || self.errors.iter().any(|e| matches!(e.severity(), DiscrepancySeverity::Critical))
    }

    /// Check if operation requires immediate attention
    #[must_use]
    pub fn requires_immediate_attention(&self) -> bool {
        self.get_summary_stats().overall_severity.requires_immediate_action()
    }

    /// Get user-friendly operation summary
    #[must_use]
    pub fn get_user_summary(&self) -> String {
        let stats = self.get_summary_stats();
        
        if stats.total_issues == 0 {
            format!("✓ {} completed successfully in {:.1}s", 
                format!("{:?}", self.operation_type), 
                stats.operation_duration.as_secs_f64())
        } else {
            format!("⚠ {} completed with {} issue(s) ({:?} severity) in {:.1}s", 
                format!("{:?}", self.operation_type),
                stats.total_issues,
                stats.overall_severity,
                stats.operation_duration.as_secs_f64())
        }
    }
}

/// Summary statistics for a guard operation context
#[derive(Debug, Clone)]
pub struct ContextSummaryStats {
    /// Total number of issues found
    pub total_issues: usize,
    /// Number of issues that can be automatically recovered
    pub recoverable_count: usize,
    /// Number of issues that require user confirmation
    pub confirmation_required: usize,
    /// Number of issues requiring manual intervention
    pub manual_intervention_required: usize,
    /// Overall severity level
    pub overall_severity: DiscrepancySeverity,
    /// Duration of the operation
    pub operation_duration: Duration,
}

/// Utility functions for creating error contexts from common scenarios
impl GuardErrorContext {
    /// Create context for a verification operation
    #[must_use]
    pub fn for_verification(
        operation_type: OperationType,
        event_sender: EventSender,
        verbosity_level: VerbosityLevel,
    ) -> Self {
        let mut context = Self::new(operation_type, event_sender, verbosity_level);
        context.add_metadata("operation_category", "verification");
        context
    }

    /// Create context for a healing operation
    #[must_use]
    pub fn for_healing(
        operation_type: OperationType,
        event_sender: EventSender,
        verbosity_level: VerbosityLevel,
    ) -> Self {
        let mut context = Self::new(operation_type, event_sender, verbosity_level);
        context.add_metadata("operation_category", "healing");
        context
    }

    /// Create context for a configuration validation operation
    #[must_use]
    pub fn for_configuration(
        operation_type: OperationType,
        event_sender: EventSender,
        verbosity_level: VerbosityLevel,
    ) -> Self {
        let mut context = Self::new(operation_type, event_sender, verbosity_level);
        context.add_metadata("operation_category", "configuration");
        context
    }
}

/// Helper trait for converting verbosity levels from user input
pub trait VerbosityLevelExt {
    /// Parse verbosity level from string
    fn from_str(s: &str) -> VerbosityLevel;
    
    /// Parse verbosity level from CLI flags (debug, verbose, quiet)
    fn from_flags(debug: bool, verbose: bool, quiet: bool) -> VerbosityLevel;
}

impl VerbosityLevelExt for VerbosityLevel {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "minimal" | "quiet" | "q" => Self::Minimal,
            "standard" | "normal" | "s" => Self::Standard,
            "detailed" | "verbose" | "v" => Self::Detailed,
            "debug" | "d" => Self::Debug,
            _ => Self::Standard,
        }
    }

    fn from_flags(debug: bool, verbose: bool, quiet: bool) -> Self {
        if debug {
            Self::Debug
        } else if verbose {
            Self::Detailed
        } else if quiet {
            Self::Minimal
        } else {
            Self::Standard
        }
    }
}
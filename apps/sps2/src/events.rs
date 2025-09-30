//! Event handling and progress display

use crate::logging::log_event_with_tracing;
use console::{style, Term};
use sps2_events::{
    events::{LifecycleEvent, LifecycleStage, LifecycleUpdateOperation},
    AppEvent, EventMessage, EventMeta, ProgressEvent,
};
use std::collections::HashMap;
use std::time::Duration;

/// Event severity levels for UI styling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSeverity {
    /// Debug information (shown only with --debug)
    Debug,
    /// Normal informational messages
    Info,
    /// Success messages
    Success,
    /// Warning messages
    Warning,
    /// Error messages
    Error,
    /// Critical errors
    Critical,
}

/// UI styling configuration
#[derive(Clone)]
pub struct UiStyle {
    /// Whether colors are supported
    colors_enabled: bool,
    /// Terminal instance for feature detection
    term: Term,
}

impl UiStyle {
    pub fn new(colors_enabled: bool) -> Self {
        Self {
            colors_enabled,
            term: Term::stdout(),
        }
    }

    /// Get styled prefix for event severity
    pub fn get_prefix(&self, severity: EventSeverity) -> String {
        if !self.colors_enabled || !self.term.features().colors_supported() {
            return match severity {
                EventSeverity::Debug => "[DEBUG]".to_string(),
                EventSeverity::Info => "[INFO]".to_string(),
                EventSeverity::Success => "[OK]".to_string(),
                EventSeverity::Warning => "[WARN]".to_string(),
                EventSeverity::Error => "[ERROR]".to_string(),
                EventSeverity::Critical => "[CRITICAL]".to_string(),
            };
        }

        // Use clean text prefixes
        match severity {
            EventSeverity::Debug => {
                format!("{}", style("[DEBUG]").dim().cyan())
            }
            EventSeverity::Info => {
                format!("{}", style("[INFO]").blue())
            }
            EventSeverity::Success => {
                format!("{}", style("[OK]").green().bold())
            }
            EventSeverity::Warning => {
                format!("{}", style("[WARN]").yellow().bold())
            }
            EventSeverity::Error => {
                format!("{}", style("[ERROR]").red().bold())
            }
            EventSeverity::Critical => {
                format!("{}", style("[CRITICAL]").red().bold().underlined())
            }
        }
    }

    /// Style message text based on severity
    pub fn style_message(&self, message: &str, severity: EventSeverity) -> String {
        if !self.colors_enabled || !self.term.features().colors_supported() {
            return message.to_string();
        }

        match severity {
            EventSeverity::Debug => style(message).dim().to_string(),
            EventSeverity::Info => message.to_string(),
            EventSeverity::Success => style(message).green().bold().to_string(),
            EventSeverity::Warning => style(message).yellow().to_string(),
            EventSeverity::Error => style(message).red().bold().to_string(),
            EventSeverity::Critical => style(message).red().bold().to_string(),
        }
    }

    /// Style message text for operations (with bold styling for important operations)
    pub fn style_operation_message(
        &self,
        message: &str,
        operation: &str,
        severity: EventSeverity,
    ) -> String {
        if !self.colors_enabled || !self.term.features().colors_supported() {
            return message.to_string();
        }

        // Apply bold styling for important operations
        let should_bold = matches!(
            operation,
            "install" | "uninstall" | "build" | "upgrade" | "rollback" | "health" | "2pc"
        );

        match severity {
            EventSeverity::Debug => style(message).dim().to_string(),
            EventSeverity::Info => {
                if should_bold {
                    style(message).bold().to_string()
                } else {
                    message.to_string()
                }
            }
            EventSeverity::Success => style(message).green().bold().to_string(),
            EventSeverity::Warning => {
                if should_bold {
                    style(message).yellow().bold().to_string()
                } else {
                    style(message).yellow().to_string()
                }
            }
            EventSeverity::Error => style(message).red().bold().to_string(),
            EventSeverity::Critical => style(message).red().bold().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct ProgressState {
    operation: String,
    total: Option<u64>,
    current: u64,
    phases: Vec<String>,
    current_phase: Option<usize>,
    last_percent_reported: Option<u8>,
    last_displayed_progress: u64,
}

/// Event handler for user feedback
pub struct EventHandler {
    /// UI styling configuration
    ui_style: UiStyle,
    /// Whether debug mode is enabled
    debug_enabled: bool,
    /// Active progress trackers keyed by progress identifier
    progress_states: HashMap<String, ProgressState>,
}

impl EventHandler {
    pub fn new(colors_enabled: bool, debug_enabled: bool) -> Self {
        Self {
            ui_style: UiStyle::new(colors_enabled),
            debug_enabled,
            progress_states: HashMap::new(),
        }
    }

    fn show_operation_message(&mut self, message: &str, operation: &str, severity: EventSeverity) {
        let prefix = self.ui_style.get_prefix(severity);
        let styled = self
            .ui_style
            .style_operation_message(message, operation, severity);
        println!("{prefix} {styled}");
    }

    fn show_message(&mut self, message: &str, severity: EventSeverity) {
        let prefix = self.ui_style.get_prefix(severity);
        let styled = self.ui_style.style_message(message, severity);
        println!("{prefix} {styled}");
    }

    /// Handle incoming event
    pub fn handle_event(&mut self, message: EventMessage) {
        // Log event with structured logging
        log_event_with_tracing(&message);

        let EventMessage { meta, event } = message;

        match event {
            // Download events
            AppEvent::Lifecycle(LifecycleEvent::Download {
                stage,
                context,
                failure,
            }) => match stage {
                LifecycleStage::Started => {
                    self.handle_download_started(
                        &meta,
                        &context.url,
                        context.package.as_deref(),
                        context.total_bytes,
                    );
                }
                LifecycleStage::Completed => {
                    self.handle_download_completed(
                        &meta,
                        &context.url,
                        context.package.as_deref(),
                        context.bytes_downloaded.unwrap_or(0),
                    );
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        self.handle_download_failed(
                            &meta,
                            &context.url,
                            context.package.as_deref(),
                            &failure_ctx,
                        );
                    }
                }
            },

            AppEvent::Lifecycle(LifecycleEvent::Acquisition {
                stage,
                context,
                failure,
            }) => {
                use sps2_events::events::LifecycleAcquisitionSource;
                match stage {
                    LifecycleStage::Started => {
                        if let LifecycleAcquisitionSource::StoreCache { hash } = &context.source {
                            self.show_operation(
                                &meta,
                                format!(
                                    "Reusing stored package {} {} (hash {})",
                                    context.package, context.version, hash
                                ),
                                "acquire",
                                EventSeverity::Info,
                            );
                        } else if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!(
                                    "Acquisition started for {} {}",
                                    context.package, context.version
                                ),
                                EventSeverity::Debug,
                            );
                        }
                    }
                    LifecycleStage::Completed => {
                        if let LifecycleAcquisitionSource::StoreCache { hash } = &context.source {
                            let size = context.size.unwrap_or(0);
                            self.show_operation(
                                &meta,
                                format!(
                                    "Prepared stored package {} {} ({}, hash {})",
                                    context.package,
                                    context.version,
                                    self.format_bytes(size),
                                    hash
                                ),
                                "acquire",
                                EventSeverity::Success,
                            );
                        } else if self.debug_enabled {
                            let size = context.size.unwrap_or(0);
                            self.show_meta_message(
                                &meta,
                                format!(
                                    "Acquisition completed for {} {} ({} bytes)",
                                    context.package, context.version, size
                                ),
                                EventSeverity::Debug,
                            );
                        }
                    }
                    LifecycleStage::Failed => {
                        if let Some(failure_ctx) = failure {
                            let message = match &context.source {
                                LifecycleAcquisitionSource::StoreCache { hash } => format!(
                                    "Failed to prepare stored package {} {} (hash {}): {}",
                                    context.package, context.version, hash, failure_ctx.message
                                ),
                                LifecycleAcquisitionSource::Remote { .. } => format!(
                                    "Acquisition failed for {} {}: {}",
                                    context.package, context.version, failure_ctx.message
                                ),
                            };
                            let severity = if failure_ctx.retryable {
                                EventSeverity::Warning
                            } else {
                                EventSeverity::Error
                            };
                            self.show_operation(&meta, message, "acquire", severity);
                        }
                    }
                }
            }

            // Install events (replaces package events)
            AppEvent::Lifecycle(LifecycleEvent::Install {
                stage,
                context,
                failure,
            }) => match stage {
                LifecycleStage::Started => {
                    self.show_operation(
                        &meta,
                        format!("Installing {} {}", context.package, context.version),
                        "install",
                        EventSeverity::Info,
                    );
                }
                LifecycleStage::Completed => {
                    let files = context.files_installed.unwrap_or(0);
                    self.show_operation(
                        &meta,
                        format!(
                            "Installed {} {} ({} files)",
                            context.package, context.version, files
                        ),
                        "install",
                        EventSeverity::Success,
                    );
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        let sps2_events::FailureContext {
                            code: _,
                            message: failure_message,
                            hint,
                            retryable,
                        } = failure_ctx;

                        let retry_text = if retryable { " (retryable)" } else { "" };
                        let mut message = format!(
                            "Failed to install {} {}{}: {}",
                            context.package, context.version, retry_text, failure_message
                        );
                        if let Some(hint) = hint.as_ref() {
                            message.push_str(&format!(" (hint: {hint})"));
                        }
                        self.show_operation(&meta, message, "install", EventSeverity::Error);
                    }
                }
            },

            AppEvent::Package(package_event) => {
                use sps2_events::{events::PackageOutcome, PackageEvent};
                match package_event {
                    PackageEvent::OperationStarted { operation } => {
                        self.show_operation(
                            &meta,
                            format!("{operation:?} started"),
                            "package",
                            EventSeverity::Info,
                        );
                    }
                    PackageEvent::OperationCompleted { operation, outcome } => {
                        let message = match outcome {
                            PackageOutcome::List { total } => {
                                format!("{operation:?} completed: {total} packages")
                            }
                            PackageOutcome::Search { query, total } => {
                                format!("Search for '{query}' matched {total} packages")
                            }
                            PackageOutcome::Health { healthy, issues } => {
                                if healthy {
                                    "Health check completed successfully".to_string()
                                } else if issues.is_empty() {
                                    "Health check reported issues".to_string()
                                } else {
                                    format!(
                                        "Health check reported {} issue(s): {}",
                                        issues.len(),
                                        issues.join(", ")
                                    )
                                }
                            }
                            PackageOutcome::SelfUpdate {
                                from,
                                to,
                                duration_ms,
                            } => {
                                format!(
                                    "Self-update completed: {from} -> {to} ({:.2}s)",
                                    duration_ms as f64 / 1000.0
                                )
                            }
                            PackageOutcome::Cleanup {
                                states_removed,
                                packages_removed,
                                duration_ms,
                            } => {
                                format!(
                                    "Cleanup removed {states_removed} states, {packages_removed} packages ({:.2}s)",
                                    duration_ms as f64 / 1000.0
                                )
                            }
                        };
                        self.show_operation(&meta, message, "package", EventSeverity::Success);
                    }
                    PackageEvent::OperationFailed { operation, failure } => {
                        let hint = failure
                            .hint
                            .as_ref()
                            .map(|h| format!(" (hint: {h})"))
                            .unwrap_or_default();
                        self.show_operation(
                            &meta,
                            format!("{operation:?} failed: {}{}", failure.message, hint),
                            "package",
                            EventSeverity::Error,
                        );
                    }
                }
            }

            // State events
            AppEvent::State(state_event) => {
                use sps2_events::StateEvent;
                match state_event {
                    StateEvent::TransitionStarted { context } => {
                        let operation = &context.operation;
                        let target = context.target;
                        self.show_operation(
                            &meta,
                            format!("State transition started ({operation} -> {target})"),
                            "state",
                            EventSeverity::Info,
                        );
                    }
                    StateEvent::TransitionCompleted { context, summary } => {
                        let source_text = context
                            .source
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "<none>".to_string());
                        let duration_text = summary
                            .and_then(|s| s.duration_ms)
                            .map(|ms| format!(" in {:.2}s", ms as f64 / 1000.0))
                            .unwrap_or_default();
                        self.show_operation(
                            &meta,
                            format!(
                                "State transition completed ({}: {source_text} -> {}){duration_text}",
                                context.operation,
                                context.target
                            ),
                            "state",
                            EventSeverity::Success,
                        );
                    }
                    StateEvent::TransitionFailed { context, failure } => {
                        self.show_operation(
                            &meta,
                            format!(
                                "State transition failed ({} -> {}): {}",
                                context.operation, context.target, failure.message
                            ),
                            "state",
                            EventSeverity::Error,
                        );
                    }
                    StateEvent::RollbackStarted { context } => {
                        self.show_operation(
                            &meta,
                            format!("Rollback started ({} -> {})", context.from, context.to),
                            "rollback",
                            EventSeverity::Info,
                        );
                    }
                    StateEvent::RollbackCompleted { context, summary } => {
                        let duration_text = summary
                            .and_then(|s| s.duration_ms)
                            .map(|ms| format!(" in {:.2}s", ms as f64 / 1000.0))
                            .unwrap_or_default();
                        self.show_operation(
                            &meta,
                            format!(
                                "Rollback completed ({} -> {}){duration_text}",
                                context.from, context.to
                            ),
                            "rollback",
                            EventSeverity::Success,
                        );
                    }
                    StateEvent::RollbackFailed { context, failure } => {
                        self.show_operation(
                            &meta,
                            format!(
                                "Rollback failed ({} -> {}): {}",
                                context.from, context.to, failure.message
                            ),
                            "rollback",
                            EventSeverity::Error,
                        );
                    }
                    StateEvent::CleanupStarted { summary } => {
                        let planned = summary.planned_states;
                        if self.debug_enabled {
                            self.show_operation(
                                &meta,
                                format!("Cleanup planned for {planned} states"),
                                "clean",
                                EventSeverity::Debug,
                            );
                        }
                    }
                    StateEvent::CleanupCompleted { summary } => {
                        let removed = summary.removed_states.unwrap_or(0);
                        let bytes = summary.space_freed_bytes.unwrap_or(0);
                        self.show_operation(
                            &meta,
                            format!(
                                "Cleanup completed: removed {} states, {} freed",
                                removed,
                                self.format_bytes(bytes)
                            ),
                            "clean",
                            EventSeverity::Success,
                        );
                    }
                    StateEvent::CleanupFailed { summary, failure } => {
                        let planned = summary.planned_states;
                        self.show_operation(
                            &meta,
                            format!(
                                "Cleanup failed after planning {planned} states: {}",
                                failure.message
                            ),
                            "clean",
                            EventSeverity::Error,
                        );
                    }
                }
            }

            // Build events
            AppEvent::Build(build_event) => {
                use sps2_events::{BuildDiagnostic, BuildEvent, LogStream, PhaseStatus};
                match build_event {
                    BuildEvent::Started { session, target } => {
                        let cache_text = if session.cache_enabled {
                            " (cache enabled)"
                        } else {
                            ""
                        };
                        self.show_operation(
                            &meta,
                            format!(
                                "Build started for {} {} using {:?}{}",
                                target.package, target.version, session.system, cache_text
                            ),
                            "build",
                            EventSeverity::Info,
                        );
                    }
                    BuildEvent::Completed {
                        target,
                        artifacts,
                        duration_ms,
                        ..
                    } => {
                        let duration = std::time::Duration::from_millis(duration_ms);
                        let artifact_summary = if artifacts.is_empty() {
                            "no artifacts produced".to_string()
                        } else {
                            format!(
                                "{} artifact{}",
                                artifacts.len(),
                                if artifacts.len() == 1 { "" } else { "s" }
                            )
                        };
                        self.show_operation(
                            &meta,
                            format!(
                                "Build completed for {} {} in {} ({artifact_summary})",
                                target.package,
                                target.version,
                                format_duration(duration)
                            ),
                            "build",
                            EventSeverity::Success,
                        );
                    }
                    BuildEvent::Failed {
                        target,
                        failure,
                        phase,
                        command,
                        ..
                    } => {
                        let mut message = format!(
                            "Build failed for {} {}: {}{}{}",
                            target.package,
                            target.version,
                            failure
                                .code
                                .as_ref()
                                .map(|code| format!("[{code}] "))
                                .unwrap_or_default(),
                            failure.message,
                            failure
                                .hint
                                .as_ref()
                                .map(|hint| format!(" (hint: {hint})"))
                                .unwrap_or_default()
                        );
                        if let Some(phase) = phase {
                            message.push_str(&format!(" during phase {phase:?}"));
                        }
                        if let Some(command) = command {
                            message.push_str(&format!(" (command: {})", command.command));
                        }
                        let severity = if failure.retryable {
                            EventSeverity::Warning
                        } else {
                            EventSeverity::Error
                        };
                        self.show_operation(&meta, message, "build", severity);
                    }
                    BuildEvent::PhaseStatus { phase, status, .. } => match status {
                        PhaseStatus::Started => {
                            self.show_operation(
                                &meta,
                                format!("Entering build phase {phase:?}"),
                                "build",
                                EventSeverity::Info,
                            );
                        }
                        PhaseStatus::Completed { duration_ms } => {
                            let duration_text = duration_ms
                                .map(|ms| {
                                    format!(
                                        " in {}",
                                        format_duration(std::time::Duration::from_millis(ms))
                                    )
                                })
                                .unwrap_or_default();
                            self.show_operation(
                                &meta,
                                format!("Completed build phase {phase:?}{duration_text}"),
                                "build",
                                EventSeverity::Success,
                            );
                        }
                    },
                    BuildEvent::Diagnostic(diag) => match diag {
                        BuildDiagnostic::Warning {
                            message, source, ..
                        } => {
                            let source_text = source
                                .as_ref()
                                .map(|s| format!(" ({s})"))
                                .unwrap_or_default();
                            self.show_operation(
                                &meta,
                                format!("Build warning: {message}{source_text}"),
                                "build",
                                EventSeverity::Warning,
                            );
                        }
                        BuildDiagnostic::LogChunk { stream, text, .. } => {
                            let prefix = match stream {
                                LogStream::Stdout => "[build]",
                                LogStream::Stderr => "[build:stderr]",
                            };
                            for line in text.lines() {
                                println!("{prefix} {line}");
                            }
                        }
                        BuildDiagnostic::CachePruned {
                            removed_items,
                            freed_bytes,
                        } => {
                            if self.debug_enabled {
                                self.show_operation(
                                    &meta,
                                    format!(
                                        "Build cache pruned: {removed_items} entries, {} freed",
                                        self.format_bytes(freed_bytes)
                                    ),
                                    "build",
                                    EventSeverity::Debug,
                                );
                            }
                        }
                    },
                }
            }

            // Resolver events
            AppEvent::Lifecycle(LifecycleEvent::Resolver {
                stage,
                context,
                failure,
            }) => match stage {
                LifecycleStage::Started => {
                    let mut parts = Vec::new();
                    let runtime_targets = context.runtime_targets.unwrap_or(0);
                    let build_targets = context.build_targets.unwrap_or(0);
                    let local_targets = context.local_targets.unwrap_or(0);

                    if runtime_targets > 0 {
                        parts.push(format!("{} runtime", runtime_targets));
                    }
                    if build_targets > 0 {
                        parts.push(format!("{} build", build_targets));
                    }
                    if local_targets > 0 {
                        parts.push(format!("{} local", local_targets));
                    }
                    if parts.is_empty() {
                        parts.push("no targets".to_string());
                    }
                    self.show_operation(
                        &meta,
                        format!("Resolving dependencies ({})", parts.join(", ")),
                        "resolve",
                        EventSeverity::Info,
                    );
                }
                LifecycleStage::Completed => {
                    let total_packages = context.total_packages.unwrap_or(0);
                    let downloaded_packages = context.downloaded_packages.unwrap_or(0);
                    let reused_packages = context.reused_packages.unwrap_or(0);
                    let duration_ms = context.duration_ms.unwrap_or(0);

                    let mut message =
                        format!("Resolved {} packages in {}ms", total_packages, duration_ms);
                    if downloaded_packages > 0 {
                        message.push_str(&format!(" • downloads: {}", downloaded_packages));
                    }
                    if reused_packages > 0 {
                        message.push_str(&format!(" • reused: {}", reused_packages));
                    }
                    self.show_operation(&meta, message, "resolve", EventSeverity::Success);
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        let code_prefix = failure_ctx
                            .code
                            .as_deref()
                            .map(|code| format!("[{code}] "))
                            .unwrap_or_default();
                        let mut message =
                            format!("Resolution failed: {code_prefix}{}", failure_ctx.message);
                        if !context.conflicting_packages.is_empty() {
                            let sample = context
                                .conflicting_packages
                                .iter()
                                .take(3)
                                .cloned()
                                .collect::<Vec<_>>();
                            message.push_str(&format!(" • conflicts: {}", sample.join(", ")));
                            if context.conflicting_packages.len() > 3 {
                                message.push_str(&format!(
                                    " (+{} more)",
                                    context.conflicting_packages.len() - 3
                                ));
                            }
                        }
                        if let Some(hint) = &failure_ctx.hint {
                            message.push_str(&format!(" (hint: {hint})"));
                        }
                        let severity = if failure_ctx.retryable {
                            EventSeverity::Warning
                        } else {
                            EventSeverity::Error
                        };
                        self.show_operation(&meta, message, "resolve", severity);
                    }
                }
            },

            // Uninstall events
            AppEvent::Lifecycle(LifecycleEvent::Uninstall {
                stage,
                context,
                failure,
            }) => match stage {
                LifecycleStage::Started => {
                    let package = context.package.as_deref().unwrap_or("<unknown>");
                    let version = context
                        .version
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    self.show_operation(
                        &meta,
                        format!("Uninstalling {} {}", package, version),
                        "uninstall",
                        EventSeverity::Info,
                    );
                }
                LifecycleStage::Completed => {
                    let package = context.package.as_deref().unwrap_or("<unknown>");
                    let version = context
                        .version
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    let files = context.files_removed.unwrap_or(0);
                    self.show_operation(
                        &meta,
                        format!("Uninstalled {} {} ({} files)", package, version, files),
                        "uninstall",
                        EventSeverity::Success,
                    );
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        let package = context.package.as_deref().unwrap_or("<unknown>");
                        let version = context
                            .version
                            .as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "<unknown>".to_string());
                        let retry_suffix = if failure_ctx.retryable {
                            " (retryable)"
                        } else {
                            ""
                        };
                        let code_prefix = failure_ctx
                            .code
                            .as_deref()
                            .map(|c| format!("[{c}] "))
                            .unwrap_or_default();
                        let mut message = format!(
                            "Failed to uninstall {} {}{}: {code_prefix}{}",
                            package, version, retry_suffix, failure_ctx.message
                        );
                        if let Some(hint) = &failure_ctx.hint {
                            message.push_str(&format!(" (hint: {hint})"));
                        }
                        let severity = if failure_ctx.retryable {
                            EventSeverity::Warning
                        } else {
                            EventSeverity::Error
                        };
                        self.show_operation(&meta, message, "uninstall", severity);
                    }
                }
            },

            // Update events
            AppEvent::Lifecycle(LifecycleEvent::Update {
                stage,
                context,
                failure,
            }) => match stage {
                LifecycleStage::Started => {
                    let op_label = match context.operation {
                        LifecycleUpdateOperation::Update => "update",
                        LifecycleUpdateOperation::Upgrade => "upgrade",
                        LifecycleUpdateOperation::Downgrade => "downgrade",
                        LifecycleUpdateOperation::Reinstall => "reinstall",
                    };
                    let requested = context.requested.as_deref().unwrap_or(&[]);
                    let total_targets = context.total_targets.unwrap_or(0);

                    let target_text = if requested.is_empty() {
                        format!("all ({total_targets})")
                    } else if requested.len() == 1 {
                        requested[0].clone()
                    } else {
                        format!("{} packages", requested.len())
                    };
                    self.show_operation(
                        &meta,
                        format!("Starting {op_label} for {target_text}"),
                        "update",
                        EventSeverity::Info,
                    );
                }
                LifecycleStage::Completed => {
                    let op_label = match context.operation {
                        LifecycleUpdateOperation::Update => "update",
                        LifecycleUpdateOperation::Upgrade => "upgrade",
                        LifecycleUpdateOperation::Downgrade => "downgrade",
                        LifecycleUpdateOperation::Reinstall => "reinstall",
                    };
                    let updated = context.updated.as_deref().unwrap_or(&[]);
                    let skipped = context.skipped.unwrap_or(0);
                    let size_difference = context.size_difference.unwrap_or(0);
                    let duration = context
                        .duration
                        .unwrap_or_else(|| std::time::Duration::from_secs(0));

                    let mut message = if updated.is_empty() {
                        format!("No packages required {op_label}")
                    } else if updated.len() == 1 {
                        format!(
                            "{op_label}d {} to {}",
                            updated[0].package, updated[0].to_version
                        )
                    } else {
                        format!("{op_label}d {} packages", updated.len())
                    };
                    if skipped > 0 {
                        message.push_str(&format!(" • skipped {skipped}"));
                    }
                    if size_difference != 0 {
                        message.push_str(&format!(" • size Δ {size_difference} bytes"));
                    }
                    message.push_str(&format!(" ({:.1}s)", duration.as_secs_f32()));
                    self.show_operation(&meta, message, "update", EventSeverity::Success);
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        let op_label = match context.operation {
                            LifecycleUpdateOperation::Update => "update",
                            LifecycleUpdateOperation::Upgrade => "upgrade",
                            LifecycleUpdateOperation::Downgrade => "downgrade",
                            LifecycleUpdateOperation::Reinstall => "reinstall",
                        };
                        let code_prefix = failure_ctx
                            .code
                            .as_deref()
                            .map(|c| format!("[{c}] "))
                            .unwrap_or_default();
                        let mut message =
                            format!("{op_label} failed: {code_prefix}{}", failure_ctx.message);

                        let failed = context.failed.as_deref().unwrap_or(&[]);
                        let updated = context.updated.as_deref().unwrap_or(&[]);

                        if !failed.is_empty() {
                            let sample = failed.iter().take(3).cloned().collect::<Vec<_>>();
                            message.push_str(&format!(" • failed: {}", sample.join(", ")));
                            if failed.len() > 3 {
                                message.push_str(&format!(" (+{} more)", failed.len() - 3));
                            }
                        }
                        if !updated.is_empty() {
                            message.push_str(&format!(
                                " • completed {} before failure",
                                updated.len()
                            ));
                        }
                        if let Some(hint) = &failure_ctx.hint {
                            message.push_str(&format!(" (hint: {hint})"));
                        }
                        let severity = if failure_ctx.retryable {
                            EventSeverity::Warning
                        } else {
                            EventSeverity::Error
                        };
                        self.show_operation(&meta, message, "update", severity);
                    }
                }
            },

            // General operation events (handles repository, search, list, cleanup, rollback, health)
            AppEvent::General(general_event) => {
                use sps2_events::GeneralEvent;
                match general_event {
                    GeneralEvent::OperationStarted { operation } => {
                        // Map operation names to appropriate display messages and icons
                        let (message, icon) = match operation.to_lowercase().as_str() {
                            op if op.contains("sync") || op.contains("repo") => {
                                ("Syncing repository index", "sync")
                            }
                            op if op.contains("search") => ("Searching packages", "search"),
                            op if op.contains("list") => ("Listing installed packages", "list"),
                            op if op.contains("clean") => ("Cleaning up system", "clean"),
                            op if op.contains("rollback") => ("Rolling back system", "rollback"),
                            op if op.contains("health") => ("Checking system health", "health"),
                            _ => (operation.as_str(), "operation"),
                        };
                        self.show_operation(&meta, message.to_string(), icon, EventSeverity::Info);
                    }
                    GeneralEvent::OperationCompleted { operation, success } => {
                        let severity = if success {
                            EventSeverity::Success
                        } else {
                            EventSeverity::Warning
                        };
                        let (message, icon) = match operation.to_lowercase().as_str() {
                            op if op.contains("sync") || op.contains("repo") => {
                                ("Repository sync completed", "sync")
                            }
                            op if op.contains("search") => ("Search completed", "search"),
                            op if op.contains("list") => ("Package listing completed", "list"),
                            op if op.contains("clean") => ("System cleanup completed", "clean"),
                            op if op.contains("rollback") => ("Rollback completed", "rollback"),
                            op if op.contains("health") => ("Health check completed", "health"),
                            _ => (operation.as_str(), "operation"),
                        };
                        self.show_operation(&meta, message.to_string(), icon, severity);
                    }
                    GeneralEvent::OperationFailed { operation, failure } => {
                        let mut text = match &failure.code {
                            Some(code) => format!("[{code}] {}", failure.message),
                            None => failure.message.clone(),
                        };
                        if let Some(hint) = &failure.hint {
                            text.push_str(&format!(" (hint: {hint})"));
                        }
                        let severity = if failure.retryable {
                            EventSeverity::Warning
                        } else {
                            EventSeverity::Error
                        };
                        self.show_operation(&meta, text, &operation, severity);
                    }
                    GeneralEvent::Warning { message, .. } => {
                        self.show_meta_message(&meta, message, EventSeverity::Warning);
                    }
                    GeneralEvent::Error { message, .. } => {
                        self.show_meta_message(&meta, message, EventSeverity::Error);
                    }
                    GeneralEvent::DebugLog { message, context } => {
                        if self.debug_enabled {
                            if context.is_empty() {
                                self.show_meta_message(&meta, message, EventSeverity::Debug);
                            } else {
                                let context_str = context
                                    .iter()
                                    .map(|(k, v)| format!("{k}={v}"))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                self.show_meta_message(
                                    &meta,
                                    format!("{message} ({context_str})"),
                                    EventSeverity::Debug,
                                );
                            }
                        }
                    }
                    GeneralEvent::CheckModePreview {
                        operation,
                        action,
                        details,
                    } => {
                        // Only show check mode preview in debug mode to reduce verbosity
                        if self.debug_enabled {
                            self.show_check_preview(&operation, &action, &details);
                        }
                    }
                    GeneralEvent::CheckModeSummary {
                        operation,
                        total_changes,
                        categories,
                    } => {
                        self.show_check_summary(&operation, &total_changes, &categories);
                    }
                }
            }

            AppEvent::Qa(qa_event) => {
                use sps2_events::{events::QaCheckStatus, QaEvent};
                match qa_event {
                    QaEvent::PipelineStarted { target, level } => {
                        self.show_operation(
                            &meta,
                            format!(
                                "Starting QA pipeline for {} {} (level: {:?})",
                                target.package, target.version, level
                            ),
                            "qa",
                            EventSeverity::Info,
                        );
                    }
                    QaEvent::PipelineCompleted {
                        target,
                        total_checks,
                        passed,
                        failed,
                        duration_ms,
                    } => {
                        let duration_text = format!("{:.2}s", duration_ms as f64 / 1000.0);
                        let message = if failed == 0 {
                            format!(
                                "QA pipeline completed for {} {}: {passed}/{total_checks} checks passed ({duration_text})",
                                target.package, target.version
                            )
                        } else {
                            format!(
                                "QA pipeline completed for {} {}: {passed}/{total_checks} passed, {failed} failed ({duration_text})",
                                target.package, target.version
                            )
                        };
                        let severity = if failed == 0 {
                            EventSeverity::Success
                        } else {
                            EventSeverity::Warning
                        };
                        self.show_operation(&meta, message, "qa", severity);
                    }
                    QaEvent::PipelineFailed { target, failure } => {
                        let hint_text = failure
                            .hint
                            .as_ref()
                            .map(|h| format!(" (hint: {h})"))
                            .unwrap_or_default();
                        self.show_operation(
                            &meta,
                            format!(
                                "QA pipeline failed for {} {}: {}{}",
                                target.package, target.version, failure.message, hint_text
                            ),
                            "qa",
                            EventSeverity::Error,
                        );
                    }
                    QaEvent::CheckEvaluated { summary, .. } => {
                        let severity = match summary.status {
                            QaCheckStatus::Passed => EventSeverity::Info,
                            QaCheckStatus::Failed => EventSeverity::Error,
                            QaCheckStatus::Skipped => EventSeverity::Debug,
                        };
                        let findings_text = if summary.findings.is_empty() {
                            String::from("no findings")
                        } else {
                            format!("{} finding(s)", summary.findings.len())
                        };
                        let duration_text = summary
                            .duration_ms
                            .map(|ms| format!(" in {:.2}s", ms as f64 / 1000.0))
                            .unwrap_or_default();
                        self.show_operation(
                            &meta,
                            format!(
                                "Check {} ({}) -> {:?}: {findings_text}{duration_text}",
                                summary.name, summary.category, summary.status
                            ),
                            "qa",
                            severity,
                        );
                    }
                }
            }

            AppEvent::Progress(progress_event) => {
                self.handle_progress_event(&meta, progress_event);
            }

            AppEvent::Guard(guard_event) => {
                use sps2_events::{GuardEvent, GuardScope, GuardSeverity};

                fn scope_label(scope: &GuardScope) -> String {
                    match scope {
                        GuardScope::System => "system".to_string(),
                        GuardScope::Package { name, version } => version
                            .as_ref()
                            .map(|v| format!("package {name}:{v}"))
                            .unwrap_or_else(|| format!("package {name}")),
                        GuardScope::Path { path } => format!("path {path}"),
                        GuardScope::State { id } => format!("state {id}"),
                        GuardScope::Custom { description } => description.clone(),
                    }
                }

                let format_failure = |failure: &sps2_events::FailureContext| {
                    format!(
                        "{}{}{}",
                        failure
                            .code
                            .as_ref()
                            .map(|code| format!("[{code}] "))
                            .unwrap_or_default(),
                        failure.message,
                        failure
                            .hint
                            .as_ref()
                            .map(|hint| format!(" (hint: {hint})"))
                            .unwrap_or_default()
                    )
                };

                match guard_event {
                    GuardEvent::VerificationStarted {
                        scope,
                        level,
                        targets,
                        ..
                    } => {
                        let files_info = targets
                            .files
                            .map(|files| format!(", {files} files"))
                            .unwrap_or_default();
                        self.show_operation(
                            &meta,
                            format!(
                                "Starting {:?} verification ({}, {} packages{})",
                                level,
                                scope_label(&scope),
                                targets.packages,
                                files_info
                            ),
                            "verify",
                            EventSeverity::Info,
                        );
                    }
                    GuardEvent::VerificationCompleted {
                        scope,
                        discrepancies,
                        metrics,
                        ..
                    } => {
                        let summary = format!(
                            "coverage {:.1}%, cache hits {:.1}%",
                            metrics.coverage_percent,
                            metrics.cache_hit_rate * 100.0
                        );
                        if discrepancies == 0 {
                            self.show_operation(
                                &meta,
                                format!(
                                    "Verification succeeded for {} ({summary}, {}ms)",
                                    scope_label(&scope),
                                    metrics.duration_ms
                                ),
                                "verify",
                                EventSeverity::Success,
                            );
                        } else {
                            self.show_operation(
                                &meta,
                                format!(
                                    "Verification found {} issue(s) for {} ({summary}, {}ms)",
                                    discrepancies,
                                    scope_label(&scope),
                                    metrics.duration_ms
                                ),
                                "verify",
                                EventSeverity::Warning,
                            );
                        }
                    }
                    GuardEvent::VerificationFailed { scope, failure, .. } => {
                        let severity = if failure.retryable {
                            EventSeverity::Warning
                        } else {
                            EventSeverity::Error
                        };
                        self.show_operation(
                            &meta,
                            format!(
                                "Verification failed for {}: {}",
                                scope_label(&scope),
                                format_failure(&failure)
                            ),
                            "verify",
                            severity,
                        );
                    }
                    GuardEvent::HealingStarted { plan, .. } => {
                        let manual = plan.manual_only;
                        let confirmation = plan.confirmation_required;
                        let auto = plan.auto_heal;
                        self.show_operation(
                            &meta,
                            format!(
                                "Healing started: {} total (auto: {}, confirm: {}, manual: {})",
                                plan.total, auto, confirmation, manual
                            ),
                            "verify",
                            EventSeverity::Info,
                        );
                    }
                    GuardEvent::HealingCompleted {
                        healed,
                        failed,
                        duration_ms,
                        ..
                    } => {
                        let severity = if failed > 0 {
                            EventSeverity::Warning
                        } else {
                            EventSeverity::Success
                        };
                        self.show_operation(
                            &meta,
                            format!(
                                "Healing finished: {healed} healed, {failed} failed ({duration_ms}ms)"
                            ),
                            "verify",
                            severity,
                        );
                    }
                    GuardEvent::HealingFailed {
                        failure, healed, ..
                    } => {
                        let severity = if failure.retryable {
                            EventSeverity::Warning
                        } else {
                            EventSeverity::Error
                        };
                        self.show_operation(
                            &meta,
                            format!(
                                "Healing failed after {healed} success(es): {}",
                                format_failure(&failure)
                            ),
                            "verify",
                            severity,
                        );
                    }
                    GuardEvent::DiscrepancyReported { discrepancy, .. } => {
                        let severity = match discrepancy.severity {
                            GuardSeverity::Critical => EventSeverity::Critical,
                            GuardSeverity::High => EventSeverity::Error,
                            GuardSeverity::Medium => EventSeverity::Warning,
                            GuardSeverity::Low => EventSeverity::Info,
                        };
                        let location = discrepancy
                            .location
                            .as_ref()
                            .map(|loc| format!(" ({loc})"))
                            .unwrap_or_default();
                        let package_info = match (&discrepancy.package, &discrepancy.version) {
                            (Some(pkg), Some(ver)) => format!(" [{pkg}:{ver}]"),
                            (Some(pkg), None) => format!(" [{pkg}]"),
                            _ => String::new(),
                        };
                        self.show_meta_message(
                            &meta,
                            format!(
                                "{}{}: {}{}",
                                discrepancy.kind, package_info, discrepancy.message, location
                            ),
                            severity,
                        );
                    }
                }
            }

            // Catch-all for other events (silently ignore for now)
            _ => {
                self.show_unhandled_event(&meta, &event);
            }
        }
    }

    fn handle_progress_event(&mut self, meta: &EventMeta, progress_event: ProgressEvent) {
        match progress_event {
            ProgressEvent::Started {
                id,
                operation,
                total,
                phases,
                ..
            } => {
                let phase_names = phases.into_iter().map(|p| p.name).collect();
                let state = ProgressState {
                    operation: operation.clone(),
                    total,
                    current: 0,
                    phases: phase_names,
                    current_phase: None,
                    last_percent_reported: None,
                    last_displayed_progress: 0,
                };
                self.progress_states.insert(id.clone(), state);
                self.show_meta_message(meta, format!("Started {operation}"), EventSeverity::Info);
            }
            ProgressEvent::Updated {
                id,
                current,
                total,
                phase,
                speed,
                eta,
                ..
            } => {
                if let Some(state) = self.progress_states.get_mut(&id) {
                    state.current = current;
                    if let Some(phase_index) = phase {
                        state.current_phase = Some(phase_index);
                    }
                    if total.is_some() {
                        state.total = total;
                    }

                    let mut message = None;
                    if let Some(total) = state.total.filter(|total| *total > 0) {
                        let percent = ((current as f64 / total as f64) * 100.0)
                            .clamp(0.0, 100.0)
                            .round() as u8;
                        let should_report = state
                            .last_percent_reported
                            .is_none_or(|last| percent >= last.saturating_add(5) || percent == 100);
                        if should_report {
                            state.last_percent_reported = Some(percent);
                            let mut text = format!("{} {percent}%", state.operation);
                            if let Some(phase_idx) = state.current_phase {
                                if let Some(name) = state.phases.get(phase_idx) {
                                    text.push_str(&format!(" ({name})"));
                                }
                            }
                            if let Some(speed) = speed {
                                text.push_str(&format!(" • {speed:.1}/s"));
                            }
                            if let Some(eta) = eta {
                                text.push_str(&format!(" • eta {}", format_duration(eta)));
                            }
                            message = Some(text);
                        }
                    } else if current >= state.last_displayed_progress + 10 {
                        state.last_displayed_progress = current;
                        let mut text = format!("{} progress {}", state.operation, current);
                        if let Some(phase_idx) = state.current_phase {
                            if let Some(name) = state.phases.get(phase_idx) {
                                text.push_str(&format!(" ({name})"));
                            }
                        }
                        message = Some(text);
                    }

                    if let Some(text) = message {
                        self.show_meta_message(meta, text, EventSeverity::Info);
                    }
                } else if self.debug_enabled {
                    self.show_meta_message(
                        meta,
                        format!("Progress update for unknown tracker {id}: {current}/{total:?}"),
                        EventSeverity::Debug,
                    );
                }
            }
            ProgressEvent::PhaseChanged {
                id,
                phase,
                phase_name,
            } => {
                if let Some(operation) = self.progress_states.get_mut(&id).map(|state| {
                    state.current_phase = Some(phase);
                    state.operation.clone()
                }) {
                    self.show_meta_message(
                        meta,
                        format!("{operation} → phase {phase_name}"),
                        EventSeverity::Info,
                    );
                } else if self.debug_enabled {
                    self.show_meta_message(
                        meta,
                        format!("Phase change for unknown tracker {id}: {phase_name}"),
                        EventSeverity::Debug,
                    );
                }
            }
            ProgressEvent::Completed {
                id,
                duration,
                total_processed,
                ..
            } => {
                let message = if let Some(state) = self.progress_states.remove(&id) {
                    let mut text = format!(
                        "{} completed in {}",
                        state.operation,
                        format_duration(duration)
                    );
                    if total_processed > 0 {
                        text.push_str(&format!(" • processed {total_processed}"));
                    }
                    text
                } else {
                    format!("Progress {id} completed in {}", format_duration(duration))
                };
                self.show_meta_message(meta, message, EventSeverity::Success);
            }
            ProgressEvent::Failed {
                id,
                failure,
                partial_duration,
                ..
            } => {
                let message = if let Some(state) = self.progress_states.remove(&id) {
                    format!(
                        "{} failed after {}: {}{}{}",
                        state.operation,
                        format_duration(partial_duration),
                        failure
                            .code
                            .as_ref()
                            .map(|c| format!("[{c}] "))
                            .unwrap_or_default(),
                        failure.message,
                        failure
                            .hint
                            .as_ref()
                            .map(|h| format!(" (hint: {h})"))
                            .unwrap_or_default()
                    )
                } else {
                    format!(
                        "Progress {id} failed after {}: {}{}{}",
                        format_duration(partial_duration),
                        failure
                            .code
                            .as_ref()
                            .map(|c| format!("[{c}] "))
                            .unwrap_or_default(),
                        failure.message,
                        failure
                            .hint
                            .as_ref()
                            .map(|h| format!(" (hint: {h})"))
                            .unwrap_or_default()
                    )
                };
                let severity = if failure.retryable {
                    EventSeverity::Warning
                } else {
                    EventSeverity::Error
                };
                self.show_meta_message(meta, message, severity);
            }
            ProgressEvent::Paused { id, reason, .. } => {
                if self.debug_enabled {
                    self.show_meta_message(
                        meta,
                        format!("Progress {id} paused: {reason}"),
                        EventSeverity::Debug,
                    );
                }
            }
            ProgressEvent::Resumed { id, pause_duration } => {
                if self.debug_enabled {
                    self.show_meta_message(
                        meta,
                        format!(
                            "Progress {id} resumed after {}",
                            format_duration(pause_duration)
                        ),
                        EventSeverity::Debug,
                    );
                }
            }
            ProgressEvent::ChildStarted {
                parent_id,
                child_id,
                operation,
                ..
            } => {
                if self.debug_enabled {
                    self.show_meta_message(
                        meta,
                        format!("Progress {parent_id} -> child {child_id} started: {operation}"),
                        EventSeverity::Debug,
                    );
                }
            }
            ProgressEvent::ChildCompleted {
                parent_id,
                child_id,
                success,
            } => {
                if self.debug_enabled {
                    let status = if success { "succeeded" } else { "failed" };
                    self.show_meta_message(
                        meta,
                        format!("Progress {parent_id} -> child {child_id} {status}"),
                        EventSeverity::Debug,
                    );
                }
            }
        }
    }

    fn show_operation(
        &mut self,
        meta: &EventMeta,
        message: String,
        operation: &str,
        severity: EventSeverity,
    ) {
        let formatted = self.decorate_message(meta, message);
        self.show_operation_message(&formatted, operation, severity);
    }

    fn show_meta_message(&mut self, meta: &EventMeta, message: String, severity: EventSeverity) {
        let formatted = self.decorate_message(meta, message);
        self.show_message(&formatted, severity);
    }

    fn decorate_message(&self, meta: &EventMeta, message: String) -> String {
        let mut output = message;
        if let Some(correlation) = &meta.correlation_id {
            output = format!("[{correlation}] {output}");
        }
        if self.debug_enabled {
            output = format!("{output} (event_id={})", meta.event_id);
        }
        output
    }

    /// Show unhandled event message
    fn show_unhandled_event(&mut self, meta: &EventMeta, event: &AppEvent) {
        if self.debug_enabled {
            let event_name = match event {
                AppEvent::Lifecycle(lifecycle_event) => match lifecycle_event {
                    LifecycleEvent::Acquisition { .. } => "Acquisition",
                    LifecycleEvent::Download { .. } => "Download",
                    LifecycleEvent::Install { .. } => "Install",
                    LifecycleEvent::Resolver { .. } => "Resolver",
                    LifecycleEvent::Repo { .. } => "Repo",
                    LifecycleEvent::Uninstall { .. } => "Uninstall",
                    LifecycleEvent::Update { .. } => "Update",
                },
                AppEvent::Audit(_) => "Audit",
                AppEvent::Build(_) => "Build",
                AppEvent::General(_) => "General",
                AppEvent::Guard(_) => "Guard",
                AppEvent::Package(_) => "Package",
                AppEvent::Progress(_) => "Progress",
                AppEvent::Qa(_) => "Qa",
                AppEvent::State(_) => "State",
                AppEvent::Platform(_) => "Platform",
            };
            self.show_meta_message(
                meta,
                format!("Unhandled event in domain \"{event_name}\": {event:?}"),
                EventSeverity::Debug,
            );
        }
    }

    /// Handle download started event
    fn handle_download_started(
        &mut self,
        meta: &EventMeta,
        url: &str,
        package: Option<&str>,
        total_bytes: Option<u64>,
    ) {
        let filename = url.split('/').next_back().unwrap_or(url);
        let name = package.unwrap_or(filename);
        let size_info = if let Some(total) = total_bytes {
            format!(" ({})", self.format_bytes(total))
        } else {
            String::new()
        };

        self.show_operation(
            meta,
            format!("Downloading {name}{size_info}"),
            "download",
            EventSeverity::Info,
        );
    }

    /// Handle download completed event
    fn handle_download_completed(
        &mut self,
        meta: &EventMeta,
        url: &str,
        package: Option<&str>,
        bytes_downloaded: u64,
    ) {
        let filename = url.split('/').next_back().unwrap_or(url);
        let name = package.unwrap_or(filename);
        self.show_operation(
            meta,
            format!(
                "Finished downloading {name} ({} fetched)",
                self.format_bytes(bytes_downloaded)
            ),
            "download",
            EventSeverity::Success,
        );
    }

    /// Handle download failed event
    fn handle_download_failed(
        &mut self,
        meta: &EventMeta,
        url: &str,
        package: Option<&str>,
        failure: &sps2_events::FailureContext,
    ) {
        let filename = url.split('/').next_back().unwrap_or(url);
        let name = package.unwrap_or(filename);
        let retry_hint = if failure.retryable {
            " (retryable)"
        } else {
            ""
        };
        let mut text = format!(
            "Download failed for {name}{retry_hint}: {}",
            failure.message
        );
        if let Some(hint) = &failure.hint {
            text.push_str(&format!(" (hint: {hint})"));
        }
        self.show_operation(meta, text, "download", EventSeverity::Error);
    }

    /// Show check mode preview
    fn show_check_preview(
        &mut self,
        _operation: &str,
        action: &str,
        details: &std::collections::HashMap<String, String>,
    ) {
        if self.ui_style.colors_enabled {
            println!("  {} {}", style("PREVIEW:").blue().bold(), action);
        } else {
            println!("  PREVIEW: {action}");
        }

        // Show relevant details
        for (key, value) in details {
            println!("    {key}: {value}");
        }
    }

    /// Show check mode summary
    fn show_check_summary(
        &mut self,
        operation: &str,
        total_changes: &usize,
        categories: &std::collections::HashMap<String, usize>,
    ) {
        if self.ui_style.colors_enabled {
            println!(
                "
{} Summary for {}:",
                style("CHECK MODE").yellow().bold(),
                operation
            );
        } else {
            println!(
                "
CHECK MODE Summary for {operation}:"
            );
        }
        println!("  Total changes: {total_changes}");

        for (category, count) in categories {
            println!("  {category}: {count}");
        }

        println!(
            "
No changes were made. Use without --check to execute."
        );
    }

    fn format_bytes(&self, bytes: u64) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
        let mut size = bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{} {}", size as u64, UNITS[unit_index])
        } else {
            format!("{:.1} {}", size, UNITS[unit_index])
        }
    }
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs == 0 {
        format!("{}ms", duration.as_millis())
    } else if secs < 60 {
        format!("{secs}s")
    } else {
        let minutes = secs / 60;
        let seconds = secs % 60;
        if minutes < 60 {
            if seconds == 0 {
                format!("{minutes}m")
            } else {
                format!("{minutes}m {seconds}s")
            }
        } else {
            let hours = minutes / 60;
            let minutes = minutes % 60;
            if minutes == 0 && seconds == 0 {
                format!("{hours}h")
            } else if seconds == 0 {
                format!("{hours}h {minutes}m")
            } else {
                format!("{hours}h {minutes}m {seconds}s")
            }
        }
    }
}

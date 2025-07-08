//! Event handling and progress display

use console::{style, Term};
use sps2_events::Event;

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

    /// Get operation icon
    pub fn get_operation_icon(&self, operation: &str) -> &'static str {
        if !self.colors_enabled || !self.term.features().colors_supported() {
            return match operation.to_lowercase().as_str() {
                op if op.contains("install") => "→",
                op if op.contains("uninstall") || op.contains("remove") => "←",
                op if op.contains("update") || op.contains("upgrade") => "↑",
                op if op.contains("build") => "⚙",
                op if op.contains("download") => "↓",
                op if op.contains("search") => "?",
                op if op.contains("sync") => "↺",
                op if op.contains("verify") || op.contains("guard") => "✓",
                op if op.contains("heal") => "+",
                op if op.contains("qa") || op.contains("audit") => "?",
                op if op.contains("2pc") => "•",
                _ => "•",
            };
        }

        match operation.to_lowercase().as_str() {
            op if op.contains("install") => "•",
            op if op.contains("uninstall") || op.contains("remove") => "•",
            op if op.contains("update") || op.contains("upgrade") => "•",
            op if op.contains("build") => "•",
            op if op.contains("download") => "•",
            op if op.contains("search") => "•",
            op if op.contains("sync") => "•",
            op if op.contains("clean") => "•",
            op if op.contains("rollback") => "•",
            op if op.contains("health") => "•",
            op if op.contains("verify") || op.contains("guard") => "•",
            op if op.contains("heal") => "•",
            op if op.contains("cache") => "•",
            op if op.contains("qa") || op.contains("audit") => "•",
            op if op.contains("2pc") => "•",
            _ => "•",
        }
    }
}

/// Event handler for user feedback
pub struct EventHandler {
    /// Output renderer for final results
    #[allow(dead_code)]
    renderer: crate::display::OutputRenderer,
    /// UI styling configuration
    ui_style: UiStyle,
    /// Whether debug mode is enabled
    debug_enabled: bool,
}

impl EventHandler {
    /// Create new event handler
    pub fn new(
        renderer: crate::display::OutputRenderer,
        colors_enabled: bool,
        debug_enabled: bool,
    ) -> Self {
        Self {
            renderer,
            ui_style: UiStyle::new(colors_enabled),
            debug_enabled,
        }
    }

    /// Handle incoming event
    pub fn handle_event(&mut self, event: Event) {
        match event {
            // Download events
            Event::DownloadStarted { url, size } => {
                self.handle_download_started(&url, size);
            }
            Event::DownloadProgress {
                url,
                bytes_downloaded,
                total_bytes,
            } => {
                self.handle_download_progress(&url, bytes_downloaded, total_bytes);
            }
            Event::DownloadCompleted { url, size: _ } => {
                self.handle_download_completed(&url);
            }
            Event::DownloadFailed { url, error } => {
                self.handle_download_failed(&url, &error);
            }

            // Package events
            Event::PackageInstalling { name, version } => {
                self.show_operation_message(
                    &format!("Installing {name} {version}"),
                    "install",
                    EventSeverity::Info,
                );
            }
            Event::PackageInstalled {
                name,
                version,
                path: _,
            } => {
                self.show_operation_message(
                    &format!("Installed {name} {version}"),
                    "install",
                    EventSeverity::Success,
                );
            }
            Event::PackageDownloaded { name, version } => {
                self.show_operation_message(
                    &format!("Downloaded {name} {version}"),
                    "download",
                    EventSeverity::Success,
                );
            }
            Event::PackageBuilding { name, version } => {
                self.show_operation_message(
                    &format!("Building {name} {version}"),
                    "build",
                    EventSeverity::Info,
                );
            }

            // State events
            Event::StateCreating { state_id } => {
                self.show_operation_message(
                    &format!("Creating state {state_id}"),
                    "state",
                    EventSeverity::Info,
                );
            }
            Event::StateTransition {
                from,
                to,
                operation: _,
            } => {
                self.show_operation_message(
                    &format!("State transition {from} -> {to}"),
                    "state",
                    EventSeverity::Info,
                );
            }

            // Build events
            Event::BuildStarting { package, version } => {
                self.show_operation_message(
                    &format!("Starting build of {package} {version}"),
                    "build",
                    EventSeverity::Info,
                );
            }
            Event::BuildCompleted {
                package,
                version,
                path,
            } => {
                self.show_operation_message(
                    &format!("Built {} {} -> {}", package, version, path.display()),
                    "build",
                    EventSeverity::Success,
                );
            }
            Event::BuildFailed {
                package,
                version,
                error,
            } => {
                self.show_operation_message(
                    &format!("Build failed for {package} {version}: {error}"),
                    "build",
                    EventSeverity::Error,
                );
            }
            Event::BuildStepStarted { package, step } => {
                self.show_operation_message(
                    &format!("{package} > {step}"),
                    "build",
                    EventSeverity::Info,
                );
            }
            Event::BuildStepOutput { package: _, line } => {
                // Display build output directly
                println!("{line}");
            }
            Event::BuildStepCompleted { package, step } => {
                self.show_operation_message(
                    &format!("{package} > {step} completed"),
                    "build",
                    EventSeverity::Success,
                );
            }
            Event::BuildCommand { package, command } => {
                self.show_operation_message(
                    &format!("{package} > {command}"),
                    "build",
                    EventSeverity::Info,
                );
            }
            Event::BuildCleaned { package } => {
                self.show_operation_message(
                    &format!("Cleaned build for {package}"),
                    "clean",
                    EventSeverity::Success,
                );
            }

            // Resolver events
            Event::DependencyResolving { package, count } => {
                if count == 1 {
                    self.show_operation_message(
                        &format!("Resolving dependencies for {package}"),
                        "resolve",
                        EventSeverity::Info,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Resolving dependencies for {count} packages"),
                        "resolve",
                        EventSeverity::Info,
                    );
                }
            }
            Event::DependencyResolved {
                package,
                version: _,
                count,
            } => {
                if count == 1 {
                    self.show_operation_message(
                        &format!("Resolved dependencies for {package}"),
                        "resolve",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Resolved {count} dependencies"),
                        "resolve",
                        EventSeverity::Success,
                    );
                }
            }

            // Operation events
            Event::InstallStarting { packages } => {
                if packages.len() == 1 {
                    // Start animated progress bar for single package installation
                    self.handle_install_started(&packages[0]);
                } else {
                    // For multiple packages, show a regular message (could be enhanced later)
                    self.show_operation_message(
                        &format!("Installing {} packages", packages.len()),
                        "install",
                        EventSeverity::Info,
                    );
                }
            }
            Event::InstallCompleted { packages, state_id } => {
                if packages.len() == 1 {
                    // Complete the animated progress bar for single package
                    self.handle_install_completed(&packages[0], &state_id);
                } else {
                    self.show_operation_message(
                        &format!(
                            "Installed {} packages (state: {})",
                            packages.len(),
                            state_id
                        ),
                        "install",
                        EventSeverity::Success,
                    );
                }
            }
            Event::UninstallStarting { packages } => {
                if packages.len() == 1 {
                    self.show_operation_message(
                        &format!("Uninstalling {}", packages[0]),
                        "uninstall",
                        EventSeverity::Info,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Uninstalling {} packages", packages.len()),
                        "uninstall",
                        EventSeverity::Info,
                    );
                }
            }
            Event::UninstallCompleted { packages, state_id } => {
                if packages.len() == 1 {
                    self.show_operation_message(
                        &format!("Uninstalled {} (state: {})", packages[0], state_id),
                        "uninstall",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!(
                            "Uninstalled {} packages (state: {})",
                            packages.len(),
                            state_id
                        ),
                        "uninstall",
                        EventSeverity::Success,
                    );
                }
            }
            Event::UpdateStarting { packages } => {
                if packages.len() == 1 && packages[0] == "all" {
                    self.show_operation_message(
                        "Updating all packages",
                        "update",
                        EventSeverity::Info,
                    );
                } else if packages.len() == 1 {
                    self.show_operation_message(
                        &format!("Updating {}", packages[0]),
                        "update",
                        EventSeverity::Info,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Updating {} packages", packages.len()),
                        "update",
                        EventSeverity::Info,
                    );
                }
            }
            Event::UpdateCompleted { packages, state_id } => {
                if packages.is_empty() {
                    self.show_operation_message(
                        &format!("No updates available (state: {state_id})"),
                        "update",
                        EventSeverity::Info,
                    );
                } else if packages.len() == 1 {
                    self.show_operation_message(
                        &format!("Updated {} (state: {})", packages[0], state_id),
                        "update",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Updated {} packages (state: {})", packages.len(), state_id),
                        "update",
                        EventSeverity::Success,
                    );
                }
            }
            Event::UpgradeStarting { packages } => {
                if packages.len() == 1 && packages[0] == "all" {
                    self.show_operation_message(
                        "Upgrading all packages",
                        "upgrade",
                        EventSeverity::Info,
                    );
                } else if packages.len() == 1 {
                    self.show_operation_message(
                        &format!("Upgrading {}", packages[0]),
                        "upgrade",
                        EventSeverity::Info,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Upgrading {} packages", packages.len()),
                        "upgrade",
                        EventSeverity::Info,
                    );
                }
            }
            Event::UpgradeCompleted { packages, state_id } => {
                if packages.is_empty() {
                    self.show_operation_message(
                        &format!("No upgrades available (state: {state_id})"),
                        "upgrade",
                        EventSeverity::Info,
                    );
                } else if packages.len() == 1 {
                    self.show_operation_message(
                        &format!("Upgraded {} (state: {})", packages[0], state_id),
                        "upgrade",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Upgraded {} packages (state: {})", packages.len(), state_id),
                        "upgrade",
                        EventSeverity::Success,
                    );
                }
            }

            // Repository events
            Event::RepoSyncStarting => {
                self.show_operation_message(
                    "Syncing repository index",
                    "sync",
                    EventSeverity::Info,
                );
            }
            Event::RepoSyncCompleted {
                packages_updated,
                duration_ms,
            } => {
                if packages_updated == 0 {
                    self.show_operation_message(
                        &format!("Repository index up to date ({duration_ms}ms)"),
                        "sync",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Updated {packages_updated} packages ({duration_ms}ms)"),
                        "sync",
                        EventSeverity::Success,
                    );
                }
            }

            // Search events
            Event::SearchStarting { query } => {
                self.show_operation_message(
                    &format!("Searching for '{query}'"),
                    "search",
                    EventSeverity::Info,
                );
            }
            Event::SearchCompleted { query: _, count } => {
                self.show_operation_message(
                    &format!("Found {count} packages"),
                    "search",
                    EventSeverity::Success,
                );
            }

            // List events
            Event::ListStarting => {
                self.show_operation_message(
                    "Listing installed packages",
                    "list",
                    EventSeverity::Info,
                );
            }
            Event::ListCompleted { count } => {
                self.show_operation_message(
                    &format!("Found {count} installed packages"),
                    "list",
                    EventSeverity::Success,
                );
            }

            // Cleanup events
            Event::CleanupStarting => {
                self.show_operation_message("Cleaning up system", "clean", EventSeverity::Info);
            }
            Event::CleanupCompleted {
                states_removed,
                packages_removed,
                duration_ms,
            } => {
                self.show_operation_message(
                    &format!(
                        "Cleaned {states_removed} states and {packages_removed} packages ({duration_ms}ms)"
                    ),
                    "clean",
                    EventSeverity::Success,
                );
            }

            // Rollback events
            Event::RollbackStarting { target_state } => {
                self.show_operation_message(
                    &format!("Rolling back to state {target_state}"),
                    "rollback",
                    EventSeverity::Info,
                );
            }
            Event::RollbackCompleted {
                target_state,
                duration_ms,
            } => {
                self.show_operation_message(
                    &format!("Rolled back to {target_state} ({duration_ms}ms)"),
                    "rollback",
                    EventSeverity::Success,
                );
            }

            // Health check events
            Event::HealthCheckStarting => {
                self.show_operation_message(
                    "Checking system health",
                    "health",
                    EventSeverity::Info,
                );
            }
            Event::HealthCheckCompleted { healthy, issues } => {
                if healthy {
                    self.show_operation_message("System healthy", "health", EventSeverity::Success);
                } else {
                    self.show_operation_message(
                        &format!("{} issues found", issues.len()),
                        "health",
                        EventSeverity::Warning,
                    );
                }
            }

            // Operation events
            Event::OperationStarted { operation } => {
                self.show_operation_message(
                    &operation,
                    &operation.to_lowercase(),
                    EventSeverity::Info,
                );
            }
            Event::OperationCompleted {
                operation,
                success: _,
            } => {
                self.show_operation_message(
                    &operation,
                    &operation.to_lowercase(),
                    EventSeverity::Success,
                );
            }
            Event::OperationFailed { operation, error } => {
                self.show_operation_message(
                    &format!("{operation} failed: {error}"),
                    &operation.to_lowercase(),
                    EventSeverity::Error,
                );
            }

            // Index events
            Event::IndexUpdateStarting { url } => {
                self.show_operation_message(
                    &format!("Updating index from {url}"),
                    "sync",
                    EventSeverity::Info,
                );
            }
            Event::IndexUpdateCompleted {
                packages_added,
                packages_updated,
            } => {
                self.show_operation_message(
                    &format!("Index updated: {packages_added} added, {packages_updated} updated"),
                    "sync",
                    EventSeverity::Success,
                );
            }

            // State rollback event
            Event::StateRollback { from, to } => {
                self.show_operation_message(
                    &format!("Rolled back from {from} to {to}"),
                    "rollback",
                    EventSeverity::Success,
                );
            }

            // Two-Phase Commit events
            Event::TwoPhaseCommitStarting {
                state_id,
                parent_state_id,
                operation,
            } => {
                self.show_operation_message(
                    &format!(
                        "Starting 2PC transaction: {operation} ({parent_state_id} -> {state_id})"
                    ),
                    "2pc",
                    EventSeverity::Info,
                );
            }
            Event::TwoPhaseCommitPhaseOneStarting {
                state_id,
                operation,
            } => {
                self.show_operation_message(
                    &format!("2PC Phase 1: Preparing database changes for {operation} (state: {state_id})"),
                    "2pc",
                    EventSeverity::Info,
                );
            }
            Event::TwoPhaseCommitPhaseOneCompleted {
                state_id,
                operation,
            } => {
                self.show_operation_message(
                    &format!("2PC Phase 1: Database prepared for {operation} (state: {state_id})"),
                    "2pc",
                    EventSeverity::Success,
                );
            }
            Event::TwoPhaseCommitPhaseTwoStarting {
                state_id,
                operation,
            } => {
                self.show_operation_message(
                    &format!("2PC Phase 2: Executing filesystem swap for {operation} (state: {state_id})"),
                    "2pc",
                    EventSeverity::Info,
                );
            }
            Event::TwoPhaseCommitPhaseTwoCompleted {
                state_id,
                operation,
            } => {
                self.show_operation_message(
                    &format!("2PC Phase 2: Filesystem swap completed for {operation} (state: {state_id})"),
                    "2pc",
                    EventSeverity::Success,
                );
            }
            Event::TwoPhaseCommitCompleted {
                state_id,
                parent_state_id,
                operation,
            } => {
                self.show_operation_message(
                    &format!(
                        "2PC transaction completed: {operation} ({parent_state_id} -> {state_id})"
                    ),
                    "2pc",
                    EventSeverity::Success,
                );
            }
            Event::TwoPhaseCommitFailed {
                state_id,
                operation,
                error,
                phase,
            } => {
                self.show_operation_message(
                    &format!("2PC transaction failed during {phase}: {operation} (state: {state_id}) - {error}"),
                    "2pc",
                    EventSeverity::Error,
                );
            }

            // Warning events
            Event::Warning { message, context } => {
                if let Some(context) = context {
                    self.show_warning(&format!("{message}: {context}"));
                } else {
                    self.show_warning(&message);
                }
            }

            // Error events
            Event::Error { message, details } => {
                if let Some(details) = details {
                    self.show_message(&format!("{message}: {details}"), EventSeverity::Error);
                } else {
                    self.show_message(&message, EventSeverity::Error);
                }
            }

            // Warning events
            // Event::Warning { message, context } => {
            //     if let Some(context) = context {
            //         self.show_message(
            //             &format!("{} ({})", message, context),
            //             EventSeverity::Warning,
            //         );
            //     } else {
            //         self.show_message(&message, EventSeverity::Warning);
            //     }
            // }

            // Quality Assurance events
            Event::QaCheckStarted {
                check_type,
                check_name,
            } => {
                self.show_operation_message(
                    &format!("Starting {check_type} check: {check_name}"),
                    "qa",
                    EventSeverity::Info,
                );
            }
            Event::QaCheckCompleted {
                check_type,
                check_name,
                findings_count,
                severity_counts: _,
            } => {
                if findings_count == 0 {
                    self.show_operation_message(
                        &format!("{check_type} check passed: {check_name}"),
                        "qa",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!(
                            "{check_type} check completed: {check_name} ({findings_count} findings)"
                        ),
                        "qa",
                        EventSeverity::Warning,
                    );
                }
            }
            Event::QaCheckFailed {
                check_type,
                check_name,
                error,
            } => {
                self.show_operation_message(
                    &format!("{check_type} check failed: {check_name} - {error}"),
                    "qa",
                    EventSeverity::Error,
                );
            }
            Event::QaPipelineStarted {
                package,
                version,
                qa_level,
            } => {
                self.show_operation_message(
                    &format!("Starting QA pipeline for {package} {version} (level: {qa_level})"),
                    "qa",
                    EventSeverity::Info,
                );
            }
            Event::QaPipelineCompleted {
                package,
                version,
                total_checks,
                passed,
                failed,
                duration_seconds,
            } => {
                if failed == 0 {
                    self.show_operation_message(
                        &format!(
                            "QA pipeline completed for {package} {version}: {passed}/{total_checks} checks passed ({duration_seconds}s)"
                        ),
                        "qa",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!(
                            "QA pipeline completed for {package} {version}: {passed}/{total_checks} passed, {failed} failed ({duration_seconds}s)"
                        ),
                        "qa",
                        EventSeverity::Warning,
                    );
                }
            }
            Event::QaFindingReported {
                check_type,
                severity,
                message,
                file_path,
                line,
            } => {
                let location = match (file_path, line) {
                    (Some(path), Some(line)) => format!(" ({path}:{line})"),
                    (Some(path), None) => format!(" ({path})"),
                    _ => String::new(),
                };
                let event_severity = match severity.to_lowercase().as_str() {
                    "critical" => EventSeverity::Critical,
                    "high" => EventSeverity::Error,
                    "medium" => EventSeverity::Warning,
                    "low" => EventSeverity::Info,
                    _ => EventSeverity::Info,
                };
                self.show_message(
                    &format!(
                        "[{}] {}: {}{}",
                        check_type,
                        severity.to_uppercase(),
                        message,
                        location
                    ),
                    event_severity,
                );
            }
            Event::QaReportGenerated { format, path } => {
                if let Some(path) = path {
                    self.show_operation_message(
                        &format!("QA report generated: {path} ({format})"),
                        "qa",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!("QA report generated ({format})"),
                        "qa",
                        EventSeverity::Success,
                    );
                }
            }

            // Debug events (only show if debug mode enabled)
            Event::DebugLog { message, context } => {
                if self.debug_enabled {
                    if context.is_empty() {
                        self.show_message(&message, EventSeverity::Debug);
                    } else {
                        let context_str = context
                            .iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.show_message(
                            &format!("{message} ({context_str})"),
                            EventSeverity::Debug,
                        );
                    }
                }
            }

            // Guard verification events
            Event::GuardVerificationStarted {
                operation_id: _,
                scope,
                level,
                packages_count,
                files_count,
            } => {
                let files_info = if let Some(files) = files_count {
                    format!(" ({files} files)")
                } else {
                    String::new()
                };
                self.show_operation_message(
                    &format!(
                        "Starting {level} verification: {scope} ({packages_count} packages{files_info})"
                    ),
                    "verify",
                    EventSeverity::Info,
                );
            }

            Event::GuardVerificationProgress {
                operation_id: _,
                verified_packages,
                total_packages,
                current_package,
                cache_hit_rate,
                ..
            } => {
                let package_info = if let Some(pkg) = current_package {
                    format!(" ({pkg})")
                } else {
                    String::new()
                };
                let cache_info = if let Some(rate) = cache_hit_rate {
                    format!(" [cache: {:.1}%]", rate * 100.0)
                } else {
                    String::new()
                };
                self.show_operation_message(
                    &format!(
                        "Verified {verified_packages}/{total_packages} packages{package_info}{cache_info}"
                    ),
                    "verify",
                    EventSeverity::Info,
                );
            }

            Event::GuardDiscrepancyFound {
                operation_id: _,
                discrepancy_type,
                severity,
                file_path,
                package,
                user_message,
                auto_heal_available,
                requires_confirmation,
                ..
            } => {
                let severity_level = match severity.to_lowercase().as_str() {
                    "critical" => EventSeverity::Critical,
                    "high" => EventSeverity::Error,
                    "medium" => EventSeverity::Warning,
                    "low" => EventSeverity::Info,
                    _ => EventSeverity::Warning,
                };

                let package_info = if let Some(pkg) = package {
                    format!(" [{pkg}]")
                } else {
                    String::new()
                };

                let action_info = if auto_heal_available {
                    if requires_confirmation {
                        " (auto-heal available, confirmation required)"
                    } else {
                        " (auto-heal available)"
                    }
                } else {
                    " (manual intervention required)"
                };

                self.show_message(
                    &format!(
                        "{}: {}{} - {}{}",
                        discrepancy_type.to_uppercase(),
                        severity.to_uppercase(),
                        package_info,
                        user_message,
                        action_info
                    ),
                    severity_level,
                );

                // Show file path in debug mode
                if self.debug_enabled && !file_path.is_empty() {
                    self.show_message(&format!("  File: {file_path}"), EventSeverity::Debug);
                }
            }

            Event::GuardVerificationCompleted {
                operation_id: _,
                total_discrepancies,
                by_severity,
                duration_ms,
                cache_hit_rate,
                coverage_percent,
                scope_description,
                ..
            } => {
                if total_discrepancies == 0 {
                    self.show_operation_message(
                        &format!(
                            "Verification completed: {} ({:.1}% coverage, {:.1}% cache hits, {}ms)",
                            scope_description,
                            coverage_percent,
                            cache_hit_rate * 100.0,
                            duration_ms
                        ),
                        "verify",
                        EventSeverity::Success,
                    );
                } else {
                    let severity_breakdown = by_severity
                        .iter()
                        .map(|(sev, count)| format!("{sev}: {count}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.show_operation_message(
                        &format!(
                            "Verification completed: {total_discrepancies} discrepancies found ({severity_breakdown}) ({duration_ms}ms)"
                        ),
                        "verify",
                        EventSeverity::Warning,
                    );
                }
            }

            Event::GuardVerificationFailed {
                operation_id: _,
                error,
                packages_verified,
                files_verified,
                duration_ms,
            } => {
                self.show_operation_message(
                    &format!(
                        "Verification failed after verifying {packages_verified} packages, {files_verified} files ({duration_ms}ms): {error}"
                    ),
                    "verify",
                    EventSeverity::Error,
                );
            }

            Event::GuardErrorSummary {
                operation_id: _,
                total_errors: _,
                recoverable_errors: _,
                manual_intervention_required: _,
                overall_severity,
                user_friendly_summary,
                recommended_actions,
                ..
            } => {
                let severity_level = match overall_severity.to_lowercase().as_str() {
                    "critical" => EventSeverity::Critical,
                    "high" => EventSeverity::Error,
                    "medium" => EventSeverity::Warning,
                    "low" => EventSeverity::Info,
                    _ => EventSeverity::Warning,
                };

                self.show_message(
                    &format!("Guard Summary: {user_friendly_summary}"),
                    severity_level,
                );

                if !recommended_actions.is_empty() {
                    for action in recommended_actions {
                        self.show_message(&format!("  → {action}"), EventSeverity::Info);
                    }
                }
            }

            Event::GuardHealingStarted {
                operation_id: _,
                discrepancies_count,
                auto_heal_count,
                confirmation_required_count,
                ..
            } => {
                self.show_operation_message(
                    &format!(
                        "Starting healing: {discrepancies_count} discrepancies ({auto_heal_count} auto, {confirmation_required_count} require confirmation)"
                    ),
                    "heal",
                    EventSeverity::Info,
                );
            }

            Event::GuardHealingProgress {
                operation_id: _,
                completed,
                total,
                current_operation,
                current_file,
            } => {
                let file_info = if let Some(file) = current_file {
                    format!(" ({file})")
                } else {
                    String::new()
                };
                self.show_operation_message(
                    &format!("Healing {completed}/{total}: {current_operation}{file_info}"),
                    "heal",
                    EventSeverity::Info,
                );
            }

            Event::GuardHealingResult {
                operation_id: _,
                file_path,
                success,
                healing_action,
                error,
                ..
            } => {
                if success {
                    self.show_operation_message(
                        &format!("Healed {file_path}: {healing_action}"),
                        "heal",
                        EventSeverity::Success,
                    );
                } else {
                    let error_msg = error.as_deref().unwrap_or("unknown error");
                    self.show_operation_message(
                        &format!("Failed to heal {file_path}: {error_msg}"),
                        "heal",
                        EventSeverity::Error,
                    );
                }
            }

            Event::GuardHealingCompleted {
                operation_id: _,
                healed_count,
                failed_count,
                skipped_count,
                duration_ms,
            } => {
                let total = healed_count + failed_count + skipped_count;
                if failed_count == 0 {
                    self.show_operation_message(
                        &format!(
                            "Healing completed: {healed_count}/{total} healed ({skipped_count} skipped, {duration_ms}ms)"
                        ),
                        "heal",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!(
                            "Healing completed: {healed_count} healed, {failed_count} failed, {skipped_count} skipped ({duration_ms}ms)"
                        ),
                        "heal",
                        EventSeverity::Warning,
                    );
                }
            }

            Event::GuardHealingFailed {
                operation_id: _,
                error,
                completed_healing,
                failed_healing,
                duration_ms,
            } => {
                self.show_operation_message(
                    &format!(
                        "Healing operation failed: {error} ({completed_healing} completed, {failed_healing} failed, {duration_ms}ms)"
                    ),
                    "heal",
                    EventSeverity::Error,
                );
            }

            Event::GuardCacheWarming {
                operation_id: _,
                operation_type,
                cache_entries_loading,
            } => {
                if self.debug_enabled {
                    self.show_operation_message(
                        &format!(
                            "Warming cache for {operation_type}: loading {cache_entries_loading} entries"
                        ),
                        "cache",
                        EventSeverity::Debug,
                    );
                }
            }

            Event::GuardCacheWarmingCompleted {
                operation_id: _,
                cache_entries_loaded,
                cache_hit_rate_improvement,
                duration_ms,
            } => {
                if self.debug_enabled {
                    self.show_operation_message(
                        &format!(
                            "Cache warmed: {} entries loaded, {:.1}% hit rate improvement ({}ms)",
                            cache_entries_loaded,
                            cache_hit_rate_improvement * 100.0,
                            duration_ms
                        ),
                        "cache",
                        EventSeverity::Debug,
                    );
                }
            }

            Event::GuardConfigurationValidated {
                approach,
                enabled,
                verification_level,
                auto_heal,
                validation_warnings,
            } => {
                if self.debug_enabled {
                    self.show_message(
                        &format!("Guard configuration validated: {approach} approach, enabled={enabled}, level={verification_level}, auto_heal={auto_heal}"),
                        EventSeverity::Debug,
                    );

                    for warning in validation_warnings {
                        self.show_message(&format!("  Warning: {warning}"), EventSeverity::Warning);
                    }
                }
            }

            Event::GuardConfigurationError {
                field,
                error,
                suggested_fix,
                ..
            } => {
                if let Some(fix) = suggested_fix {
                    self.show_message(
                        &format!(
                            "Configuration error in '{field}': {error} (suggested fix: {fix})"
                        ),
                        EventSeverity::Error,
                    );
                } else {
                    self.show_message(
                        &format!("Configuration error in '{field}': {error}"),
                        EventSeverity::Error,
                    );
                }
            }

            Event::GuardRecoveryAttempt {
                operation_id: _,
                error_category,
                recovery_strategy,
                attempt_number,
                max_attempts,
            } => {
                if self.debug_enabled {
                    self.show_message(
                        &format!(
                            "Recovery attempt {attempt_number}/{max_attempts} for {error_category} error: {recovery_strategy}"
                        ),
                        EventSeverity::Debug,
                    );
                }
            }

            Event::GuardRecoverySuccess {
                operation_id: _,
                error_category,
                recovery_strategy,
                attempt_number,
                recovery_duration_ms,
            } => {
                self.show_message(
                    &format!(
                        "Recovery successful for {error_category} error: {recovery_strategy} (attempt {attempt_number}, {recovery_duration_ms}ms)"
                    ),
                    EventSeverity::Success,
                );
            }

            Event::GuardRecoveryFailed {
                operation_id: _,
                error_category,
                recovery_strategy,
                attempts_made,
                final_error,
            } => {
                self.show_message(
                    &format!(
                        "Recovery failed for {error_category} error after {attempts_made} attempts using {recovery_strategy}: {final_error}"
                    ),
                    EventSeverity::Error,
                );
            }

            // Catch-all for other events (silently ignore for now)
            _ => {
                // These events are not displayed in the CLI
                // but could be logged if debug mode is enabled
                if self.debug_enabled {
                    self.show_message(&format!("Unhandled event: {event:?}"), EventSeverity::Debug);
                }
            }
        }
    }

    /// Handle download started event
    fn handle_download_started(&mut self, url: &str, size: Option<u64>) {
        let filename = url.split('/').next_back().unwrap_or(url);
        let size_info = if let Some(total) = size {
            format!(" ({})", self.format_bytes(total))
        } else {
            String::new()
        };

        self.show_operation_message(
            &format!("Downloading {filename}{size_info}"),
            "download",
            EventSeverity::Info,
        );
    }

    /// Handle download progress event
    fn handle_download_progress(&mut self, _url: &str, _bytes_downloaded: u64, _total_bytes: u64) {
        // Progress updates are now silent for fast operations
    }

    /// Handle download completed event
    fn handle_download_completed(&mut self, url: &str) {
        let filename = url.split('/').next_back().unwrap_or(url);
        self.show_operation_message(
            &format!("Downloaded {filename}"),
            "download",
            EventSeverity::Success,
        );
    }

    /// Handle download failed event
    fn handle_download_failed(&mut self, url: &str, error: &str) {
        let filename = url.split('/').next_back().unwrap_or(url);
        self.show_error(&format!("Failed to download {filename}: {error}"));
    }

    /// Handle installation started event
    fn handle_install_started(&mut self, package_name: &str) {
        self.show_operation_message(
            &format!("Installing {package_name}"),
            "install",
            EventSeverity::Info,
        );
    }

    /// Handle installation completed event
    fn handle_install_completed(&mut self, package_name: &str, state_id: &impl std::fmt::Display) {
        self.show_operation_message(
            &format!("Installed {package_name} (state: {state_id})"),
            "install",
            EventSeverity::Success,
        );
    }

    /// Show styled message based on severity
    fn show_message(&self, message: &str, severity: EventSeverity) {
        let prefix = self.ui_style.get_prefix(severity);
        let styled_message = self.ui_style.style_message(message, severity);
        let formatted = format!("{prefix} {styled_message}");
        println!("{formatted}");
    }

    /// Show operation message with appropriate icon and styling
    fn show_operation_message(&self, message: &str, operation: &str, severity: EventSeverity) {
        let icon = self.ui_style.get_operation_icon(operation);
        let styled_message = self
            .ui_style
            .style_operation_message(message, operation, severity);
        let formatted = format!("{icon} {styled_message}");
        println!("{formatted}");
    }

    /// Show warning message
    fn show_warning(&self, message: &str) {
        self.show_message(message, EventSeverity::Warning);
    }

    /// Show error message
    fn show_error(&self, message: &str) {
        self.show_message(message, EventSeverity::Error);
    }

    /// Format bytes for display
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

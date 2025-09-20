//! Event handling and progress display

use crate::logging::log_event_with_tracing;
use console::{style, Term};
use sps2_events::{AppEvent, EventMessage, EventMeta};

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

/// Event handler for user feedback
pub struct EventHandler {
    /// UI styling configuration
    ui_style: UiStyle,
    /// Whether debug mode is enabled
    debug_enabled: bool,
}

impl EventHandler {
    pub fn new(colors_enabled: bool, debug_enabled: bool) -> Self {
        Self {
            ui_style: UiStyle::new(colors_enabled),
            debug_enabled,
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
            AppEvent::Download(download_event) => {
                use sps2_events::DownloadEvent;
                match download_event {
                    DownloadEvent::Started {
                        url, total_size, ..
                    } => {
                        self.handle_download_started(&meta, &url, total_size);
                    }
                    DownloadEvent::Progress {
                        url,
                        bytes_downloaded,
                        total_bytes,
                        ..
                    } => {
                        self.handle_download_progress(&meta, &url, bytes_downloaded, total_bytes);
                    }
                    DownloadEvent::Completed { url, .. } => {
                        self.handle_download_completed(&meta, &url);
                    }
                    DownloadEvent::Failed { url, error, .. } => {
                        self.handle_download_failed(&meta, &url, &error);
                    }
                    _ => {
                        // Handle other download events with debug output
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("Download event: {download_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            // Install events (replaces package events)
            AppEvent::Install(install_event) => {
                use sps2_events::InstallEvent;
                match install_event {
                    InstallEvent::Started {
                        package, version, ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Installing {package} {version}"),
                            "install",
                            EventSeverity::Info,
                        );
                    }
                    InstallEvent::Completed {
                        package, version, ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Installed {package} {version}"),
                            "install",
                            EventSeverity::Success,
                        );
                    }
                    InstallEvent::Failed {
                        package,
                        version,
                        error,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Failed to install {package} {version}: {error}"),
                            "install",
                            EventSeverity::Error,
                        );
                    }
                    InstallEvent::StagingStarted {
                        package, version, ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Staging {package} {version}"),
                            "install",
                            EventSeverity::Info,
                        );
                    }
                    InstallEvent::ValidationStarted {
                        package, version, ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Validating {package} {version}"),
                            "install",
                            EventSeverity::Info,
                        );
                    }
                    _ => {
                        // Handle other install events with debug output
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("Install event: {install_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            // State events
            AppEvent::State(state_event) => {
                use sps2_events::StateEvent;
                match state_event {
                    StateEvent::Created {
                        state_id,
                        operation,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Created state {state_id} for {operation}"),
                            "state",
                            EventSeverity::Info,
                        );
                    }
                    StateEvent::TransitionCompleted {
                        from,
                        to,
                        operation,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("State transition completed: {operation} ({from} -> {to})"),
                            "state",
                            EventSeverity::Success,
                        );
                    }
                    StateEvent::CleanupStarted {
                        states_to_remove,
                        estimated_space_freed,
                    } => {
                        self.show_operation(
                            &meta,
                            format!(
                                "Starting cleanup: {states_to_remove} states (est. {} freed)",
                                self.format_bytes(estimated_space_freed)
                            ),
                            "clean",
                            EventSeverity::Info,
                        );
                    }
                    StateEvent::CleanupProgress {
                        states_processed,
                        total_states,
                        space_freed,
                    } => {
                        if self.debug_enabled {
                            self.show_operation(
                                &meta,
                                format!(
                                    "Cleanup progress: {states_processed}/{total_states} ({} freed)",
                                    self.format_bytes(space_freed)
                                ),
                                "clean",
                                EventSeverity::Debug,
                            );
                        }
                    }
                    StateEvent::CleanupCompleted {
                        states_pruned,
                        states_removed,
                        space_freed,
                        duration,
                    } => {
                        self.show_operation(&meta,
                            format!(
                                "Cleanup completed: pruned {states_pruned}, removed {states_removed}, {} freed ({}s)",
                                self.format_bytes(space_freed),
                                duration.as_secs()
                            ),
                            "clean",
                            EventSeverity::Success,
                        );
                    }
                    StateEvent::TwoPhaseCommitStarting {
                        state_id,
                        parent_state_id,
                        operation,
                    } => {
                        self.show_operation(&meta,
                            format!(
                                "Starting 2PC transaction: {operation} ({parent_state_id} -> {state_id})"
                            ),
                            "2pc",
                            EventSeverity::Info,
                        );
                    }
                    StateEvent::TwoPhaseCommitCompleted {
                        state_id,
                        parent_state_id,
                        operation,
                    } => {
                        self.show_operation(&meta,
                            format!(
                                "2PC transaction completed: {operation} ({parent_state_id} -> {state_id})"
                            ),
                            "2pc",
                            EventSeverity::Success,
                        );
                    }
                    _ => {
                        // Handle other state events with debug output
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("State event: {state_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            // Build events
            AppEvent::Build(build_event) => {
                use sps2_events::BuildEvent;
                match build_event {
                    BuildEvent::SessionStarted {
                        package, version, ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Starting build session for {package} {version}"),
                            "build",
                            EventSeverity::Info,
                        );
                    }
                    BuildEvent::Completed {
                        package,
                        version,
                        path,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Built {} {} -> {}", package, version, path.display()),
                            "build",
                            EventSeverity::Success,
                        );
                    }
                    BuildEvent::Failed {
                        package,
                        version,
                        error,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Build failed for {package} {version}: {error}"),
                            "build",
                            EventSeverity::Error,
                        );
                    }
                    BuildEvent::PhaseStarted { package, phase, .. } => {
                        self.show_operation(
                            &meta,
                            format!("{package} > {phase:?} phase started"),
                            "build",
                            EventSeverity::Info,
                        );
                    }
                    BuildEvent::StepOutput { line, .. } => {
                        // Display build output directly
                        println!("{line}");
                    }
                    BuildEvent::PhaseCompleted { package, phase, .. } => {
                        self.show_operation(
                            &meta,
                            format!("{package} > {phase:?} phase completed"),
                            "build",
                            EventSeverity::Success,
                        );
                    }
                    BuildEvent::CommandStarted {
                        package, command, ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("{package} > {command}"),
                            "build",
                            EventSeverity::Info,
                        );
                    }
                    BuildEvent::Cleaned { package, .. } => {
                        self.show_operation(
                            &meta,
                            format!("Cleaned build for {package}"),
                            "clean",
                            EventSeverity::Success,
                        );
                    }
                    _ => {
                        // Handle other build events with debug output
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("Build event: {build_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            // Resolver events
            AppEvent::Resolver(resolver_event) => {
                use sps2_events::ResolverEvent;
                match resolver_event {
                    ResolverEvent::ResolutionStarted {
                        runtime_deps,
                        build_deps,
                        local_files,
                        ..
                    } => {
                        self.show_operation(&meta,
                            format!(
                                "Resolving dependencies ({runtime_deps} runtime, {build_deps} build, {local_files} local)"
                            ),
                            "resolve",
                            EventSeverity::Info,
                        );
                    }
                    ResolverEvent::ResolutionCompleted { total_packages, .. } => {
                        self.show_operation(
                            &meta,
                            format!("Resolved {total_packages} dependencies successfully"),
                            "resolve",
                            EventSeverity::Success,
                        );
                    }
                    ResolverEvent::DependencyConflictDetected {
                        conflicting_packages,
                        message,
                        ..
                    } => {
                        let package_list = conflicting_packages
                            .iter()
                            .map(|(name, version)| format!("{name}:{version}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.show_operation(
                            &meta,
                            format!("Dependency conflict detected: {message} ({package_list})"),
                            "resolve",
                            EventSeverity::Warning,
                        );
                    }
                    _ => {
                        // Handle other resolver events with debug output
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("Resolver event: {resolver_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            // Uninstall events
            AppEvent::Uninstall(uninstall_event) => {
                use sps2_events::UninstallEvent;
                match uninstall_event {
                    UninstallEvent::Started {
                        package, version, ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Uninstalling {package} {version}"),
                            "uninstall",
                            EventSeverity::Info,
                        );
                    }
                    UninstallEvent::Completed {
                        package,
                        version,
                        files_removed,
                        space_freed,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!(
                                "Uninstalled {package} {version} ({files_removed} files, {} freed)",
                                self.format_bytes(space_freed)
                            ),
                            "uninstall",
                            EventSeverity::Success,
                        );
                    }
                    UninstallEvent::Failed {
                        package,
                        version,
                        error,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Failed to uninstall {package} {version}: {error}"),
                            "uninstall",
                            EventSeverity::Error,
                        );
                    }
                    _ => {
                        // Handle other uninstall events with debug output
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("Uninstall event: {uninstall_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            // Update events
            AppEvent::Update(update_event) => {
                use sps2_events::UpdateEvent;
                match update_event {
                    UpdateEvent::Started {
                        packages_specified, ..
                    } => {
                        if packages_specified.len() == 1 && packages_specified[0] == "all" {
                            self.show_operation(
                                &meta,
                                "Updating all packages".to_string(),
                                "update",
                                EventSeverity::Info,
                            );
                        } else if packages_specified.len() == 1 {
                            self.show_operation(
                                &meta,
                                format!("Updating {}", packages_specified[0]),
                                "update",
                                EventSeverity::Info,
                            );
                        } else {
                            self.show_operation(
                                &meta,
                                format!("Updating {} packages", packages_specified.len()),
                                "update",
                                EventSeverity::Info,
                            );
                        }
                    }
                    UpdateEvent::Completed {
                        packages_updated, ..
                    } => {
                        if packages_updated.is_empty() {
                            self.show_operation(
                                &meta,
                                "No packages needed updates".to_string(),
                                "update",
                                EventSeverity::Info,
                            );
                        } else if packages_updated.len() == 1 {
                            self.show_operation(
                                &meta,
                                format!(
                                    "Updated {} to {}",
                                    packages_updated[0].package, packages_updated[0].to_version
                                ),
                                "update",
                                EventSeverity::Success,
                            );
                        } else {
                            self.show_operation(
                                &meta,
                                format!("Updated {} packages", packages_updated.len()),
                                "update",
                                EventSeverity::Success,
                            );
                        }
                    }
                    UpdateEvent::PlanningStarted {
                        packages_to_check, ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Planning updates for {} packages", packages_to_check.len()),
                            "update",
                            EventSeverity::Info,
                        );
                    }
                    _ => {
                        // Handle other update events with debug output
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("Update event: {update_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

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
                    _ => {
                        // Handle other general events with debug output
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("General event: {general_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            AppEvent::Qa(qa_event) => {
                use sps2_events::QaEvent;
                match qa_event {
                    QaEvent::PipelineStarted {
                        package,
                        version,
                        qa_level,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!(
                                "Starting QA pipeline for {package} {version} (level: {qa_level})"
                            ),
                            "qa",
                            EventSeverity::Info,
                        );
                    }
                    QaEvent::PipelineCompleted {
                        package,
                        version,
                        total_checks,
                        passed,
                        failed,
                        duration_seconds,
                        ..
                    } => {
                        if failed == 0 {
                            self.show_operation(
                                &meta,
                                format!(
                                    "QA pipeline completed for {package} {version}: {passed}/{total_checks} checks passed ({duration_seconds}s)"
                                ),
                                "qa",
                                EventSeverity::Success,
                            );
                        } else {
                            self.show_operation(
                                &meta,
                                format!(
                                    "QA pipeline completed for {package} {version}: {passed}/{total_checks} passed, {failed} failed ({duration_seconds}s)"
                                ),
                                "qa",
                                EventSeverity::Warning,
                            );
                        }
                    }
                    QaEvent::CheckStarted {
                        check_type,
                        check_name,
                        ..
                    } => {
                        self.show_operation(
                            &meta,
                            format!("Starting {check_type} check: {check_name}"),
                            "qa",
                            EventSeverity::Info,
                        );
                    }
                    QaEvent::CheckCompleted {
                        check_type,
                        check_name,
                        findings_count,
                        ..
                    } => {
                        if findings_count == 0 {
                            self.show_operation(
                                &meta,
                                format!("{check_type} check passed: {check_name}"),
                                "qa",
                                EventSeverity::Success,
                            );
                        } else {
                            self.show_operation(
                                &meta,
                                format!(
                                    "{check_type} check completed: {check_name} ({findings_count} findings)"
                                ),
                                "qa",
                                EventSeverity::Warning,
                            );
                        }
                    }
                    QaEvent::FindingReported {
                        check_type,
                        severity,
                        message,
                        file_path,
                        line,
                        ..
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
                        self.show_meta_message(
                            &meta,
                            format!(
                                "[{}] {}: {}{}",
                                check_type,
                                severity.to_uppercase(),
                                message,
                                location
                            ),
                            event_severity,
                        );
                    }
                    _ => {
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("QA event: {qa_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            AppEvent::Guard(guard_event) => {
                use sps2_events::GuardEvent;
                match guard_event {
                    GuardEvent::VerificationStarted {
                        scope,
                        level,
                        packages_count,
                        files_count,
                        ..
                    } => {
                        let files_info = if let Some(files) = files_count {
                            format!(" ({files} files)")
                        } else {
                            String::new()
                        };
                        self.show_operation(&meta,
                            format!(
                                "Starting {level} verification: {scope} ({packages_count} packages{files_info})"
                            ),
                            "verify",
                            EventSeverity::Info,
                        );
                    }
                    GuardEvent::VerificationCompleted {
                        total_discrepancies,
                        by_severity,
                        duration_ms,
                        cache_hit_rate,
                        coverage_percent,
                        scope_description,
                        ..
                    } => {
                        if total_discrepancies == 0 {
                            self.show_operation(
                                &meta,
                                format!(
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
                            self.show_operation(
                                &meta,
                                format!(
                                    "Verification completed: {total_discrepancies} discrepancies found ({severity_breakdown}) ({duration_ms}ms)"
                                ),
                                "verify",
                                EventSeverity::Warning,
                            );
                        }
                    }
                    GuardEvent::DiscrepancyFound(params) => {
                        let discrepancy_type = &params.discrepancy_type;
                        let severity = &params.severity;
                        let file_path = &params.file_path;
                        let package = &params.package;
                        let user_message = &params.user_message;
                        let auto_heal_available = params.auto_heal_available;
                        let requires_confirmation = params.requires_confirmation;
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

                        self.show_meta_message(
                            &meta,
                            format!(
                                "{}: {}{} - {}{}",
                                discrepancy_type.to_uppercase(),
                                severity.to_uppercase(),
                                package_info,
                                user_message,
                                action_info
                            ),
                            severity_level,
                        );

                        if self.debug_enabled && !file_path.is_empty() {
                            self.show_meta_message(
                                &meta,
                                format!("  File: {file_path}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                    _ => {
                        if self.debug_enabled {
                            self.show_meta_message(
                                &meta,
                                format!("Guard event: {guard_event:?}"),
                                EventSeverity::Debug,
                            );
                        }
                    }
                }
            }

            // Catch-all for other events (silently ignore for now)
            _ => {
                self.show_unhandled_event(&meta, &event);
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
                AppEvent::Acquisition(_) => "Acquisition",
                AppEvent::Audit(_) => "Audit",
                AppEvent::Build(_) => "Build",
                AppEvent::Download(_) => "Download",
                AppEvent::General(_) => "General",
                AppEvent::Guard(_) => "Guard",
                AppEvent::Install(_) => "Install",
                AppEvent::Package(_) => "Package",
                AppEvent::Progress(_) => "Progress",
                AppEvent::Python(_) => "Python",
                AppEvent::Qa(_) => "Qa",
                AppEvent::Repo(_) => "Repo",
                AppEvent::Resolver(_) => "Resolver",
                AppEvent::State(_) => "State",
                AppEvent::Uninstall(_) => "Uninstall",
                AppEvent::Update(_) => "Update",
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
    fn handle_download_started(&mut self, meta: &EventMeta, url: &str, size: Option<u64>) {
        let filename = url.split('/').next_back().unwrap_or(url);
        let size_info = if let Some(total) = size {
            format!(" ({})", self.format_bytes(total))
        } else {
            String::new()
        };

        self.show_operation(
            meta,
            format!("Downloading {filename}{size_info}"),
            "download",
            EventSeverity::Info,
        );
    }

    /// Handle download progress event
    fn handle_download_progress(
        &mut self,
        _meta: &EventMeta,
        _url: &str,
        _bytes_downloaded: u64,
        _total_bytes: u64,
    ) {
        // Progress updates are now silent for fast operations
    }

    /// Handle download completed event
    fn handle_download_completed(&mut self, meta: &EventMeta, url: &str) {
        let filename = url.split('/').next_back().unwrap_or(url);
        self.show_operation(
            meta,
            format!("Finished downloading {filename}"),
            "download",
            EventSeverity::Success,
        );
    }

    /// Handle download failed event
    fn handle_download_failed(&mut self, meta: &EventMeta, url: &str, error: &str) {
        let filename = url.split('/').next_back().unwrap_or(url);
        self.show_operation(
            meta,
            format!("Download failed for {filename}: {error}"),
            "download",
            EventSeverity::Error,
        );
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

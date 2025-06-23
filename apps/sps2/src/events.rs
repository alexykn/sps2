//! Event handling and progress display

use console::{style, Emoji, Term};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sps2_events::Event;
use std::collections::HashMap;

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

        // Use emojis and colors when supported
        match severity {
            EventSeverity::Debug => {
                format!("{} {}", Emoji("🔍", "🔍"), style("DEBUG").dim().cyan())
            }
            EventSeverity::Info => {
                format!("{}  {}", Emoji("ℹ️", "i"), style("INFO").blue())
            }
            EventSeverity::Success => {
                format!("{} {}", Emoji("✅", "✓"), style("OK").green().bold())
            }
            EventSeverity::Warning => {
                format!("{} {}", Emoji("⚠️", "!"), style("WARN").yellow().bold())
            }
            EventSeverity::Error => {
                format!("{} {}", Emoji("❌", "✗"), style("ERROR").red().bold())
            }
            EventSeverity::Critical => {
                format!(
                    "{} {}",
                    Emoji("🚨", "‼"),
                    style("CRITICAL").red().bold().underlined()
                )
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
            "install" | "uninstall" | "build" | "upgrade" | "rollback" | "health"
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
                op if op.contains("qa") || op.contains("audit") => "?",
                _ => "•",
            };
        }

        match operation.to_lowercase().as_str() {
            op if op.contains("install") => "📦",
            op if op.contains("uninstall") || op.contains("remove") => "🗑️",
            op if op.contains("update") || op.contains("upgrade") => "⬆️",
            op if op.contains("build") => "🔨",
            op if op.contains("download") => "⬇️",
            op if op.contains("search") => "🔍",
            op if op.contains("sync") => "🔄",
            op if op.contains("clean") => "🧹",
            op if op.contains("rollback") => "⏪",
            op if op.contains("health") => "🩺",
            op if op.contains("qa") || op.contains("audit") => "🔍",
            _ => "•",
        }
    }
}

/// Event handler for progress display and user feedback
pub struct EventHandler {
    /// Multi-progress manager for concurrent progress bars
    multi_progress: MultiProgress,
    /// Active progress bars by URL
    download_bars: HashMap<String, ProgressBar>,
    /// Active progress bars for installations by package name
    install_bars: HashMap<String, ProgressBar>,
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
            multi_progress: MultiProgress::new(),
            download_bars: HashMap::new(),
            install_bars: HashMap::new(),
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
                // Update existing progress bar or show message if no progress bar exists
                let package_key = format!("{}-{}", name, version);
                if let Some(pb) = self.install_bars.get(&name) {
                    let styled_message = self.ui_style.style_operation_message(
                        &format!("Installing {} {}", name, version),
                        "install",
                        EventSeverity::Info,
                    );
                    pb.set_message(format!(
                        "{} {}",
                        self.ui_style.get_operation_icon("install"),
                        styled_message
                    ));
                } else {
                    // Start progress bar if it doesn't exist
                    self.handle_install_started(&package_key);
                }
            }
            Event::PackageInstalled {
                name,
                version,
                path: _,
            } => {
                // Try to complete existing progress bar, otherwise show regular message
                let package_key = format!("{}-{}", name, version);
                if self.install_bars.contains_key(&name)
                    || self.install_bars.contains_key(&package_key)
                {
                    // Complete the progress bar (try both keys)
                    if self.install_bars.contains_key(&name) {
                        self.handle_install_completed(&name, &format!("v{}", version));
                    } else {
                        self.handle_install_completed(&package_key, &format!("v{}", version));
                    }
                } else {
                    self.show_operation_message(
                        &format!("Installed {} {}", name, version),
                        "install",
                        EventSeverity::Success,
                    );
                }
            }
            Event::PackageDownloaded { name, version } => {
                self.show_operation_message(
                    &format!("Downloaded {} {}", name, version),
                    "download",
                    EventSeverity::Success,
                );
            }
            Event::PackageBuilding { name, version } => {
                self.show_operation_message(
                    &format!("Building {} {}", name, version),
                    "build",
                    EventSeverity::Info,
                );
            }

            // State events
            Event::StateCreating { state_id } => {
                self.show_operation_message(
                    &format!("Creating state {}", state_id),
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
                    &format!("State transition {} -> {}", from, to),
                    "state",
                    EventSeverity::Info,
                );
            }

            // Build events
            Event::BuildStarting { package, version } => {
                self.show_operation_message(
                    &format!("Starting build of {} {}", package, version),
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
                    &format!("Build failed for {} {}: {}", package, version, error),
                    "build",
                    EventSeverity::Error,
                );
            }
            Event::BuildStepStarted { package, step } => {
                self.show_operation_message(
                    &format!("{} > {}", package, step),
                    "build",
                    EventSeverity::Info,
                );
            }
            Event::BuildStepOutput {
                package: _,
                line: _,
            } => {
                // Build output is now printed directly to stdout/stderr
                // This event is kept for compatibility but not displayed
            }
            Event::BuildStepCompleted { package, step } => {
                self.show_operation_message(
                    &format!("{} > {} completed", package, step),
                    "build",
                    EventSeverity::Success,
                );
            }
            Event::BuildCommand { package, command } => {
                self.show_operation_message(
                    &format!("{} > {}", package, command),
                    "build",
                    EventSeverity::Info,
                );
            }
            Event::BuildCleaned { package } => {
                self.show_operation_message(
                    &format!("Cleaned build for {}", package),
                    "clean",
                    EventSeverity::Success,
                );
            }

            // Resolver events
            Event::DependencyResolving { package, count } => {
                if count == 1 {
                    self.show_operation_message(
                        &format!("Resolving dependencies for {}", package),
                        "resolve",
                        EventSeverity::Info,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Resolving dependencies for {} packages", count),
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
                        &format!("Resolved dependencies for {}", package),
                        "resolve",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Resolved {} dependencies", count),
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
                        &format!("No updates available (state: {})", state_id),
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
                        &format!("No upgrades available (state: {})", state_id),
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
                        &format!("Repository index up to date ({}ms)", duration_ms),
                        "sync",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!("Updated {} packages ({}ms)", packages_updated, duration_ms),
                        "sync",
                        EventSeverity::Success,
                    );
                }
            }

            // Search events
            Event::SearchStarting { query } => {
                self.show_operation_message(
                    &format!("Searching for '{}'", query),
                    "search",
                    EventSeverity::Info,
                );
            }
            Event::SearchCompleted { query: _, count } => {
                self.show_operation_message(
                    &format!("Found {} packages", count),
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
                    &format!("Found {} installed packages", count),
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
                        "Cleaned {} states and {} packages ({}ms)",
                        states_removed, packages_removed, duration_ms
                    ),
                    "clean",
                    EventSeverity::Success,
                );
            }

            // Rollback events
            Event::RollbackStarting { target_state } => {
                self.show_operation_message(
                    &format!("Rolling back to state {}", target_state),
                    "rollback",
                    EventSeverity::Info,
                );
            }
            Event::RollbackCompleted {
                target_state,
                duration_ms,
            } => {
                self.show_operation_message(
                    &format!("Rolled back to {} ({}ms)", target_state, duration_ms),
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
                // Check if this is an installation operation and clean up progress bars
                if operation.to_lowercase().contains("install") {
                    // Find and remove any active installation progress bars
                    let keys_to_remove: Vec<String> = self.install_bars.keys().cloned().collect();
                    for key in keys_to_remove {
                        self.handle_install_failed(&key, &error);
                    }
                } else {
                    self.show_operation_message(
                        &format!("{} failed: {}", operation, error),
                        &operation.to_lowercase(),
                        EventSeverity::Error,
                    );
                }
            }

            // Index events
            Event::IndexUpdateStarting { url } => {
                self.show_operation_message(
                    &format!("Updating index from {}", url),
                    "sync",
                    EventSeverity::Info,
                );
            }
            Event::IndexUpdateCompleted {
                packages_added,
                packages_updated,
            } => {
                self.show_operation_message(
                    &format!(
                        "Index updated: {} added, {} updated",
                        packages_added, packages_updated
                    ),
                    "sync",
                    EventSeverity::Success,
                );
            }

            // State rollback event
            Event::StateRollback { from, to } => {
                self.show_operation_message(
                    &format!("Rolled back from {} to {}", from, to),
                    "rollback",
                    EventSeverity::Success,
                );
            }

            // Warning events
            Event::Warning { message, context } => {
                if let Some(context) = context {
                    self.show_warning(&format!("{}: {}", message, context));
                } else {
                    self.show_warning(&message);
                }
            }

            // Error events
            Event::Error { message, details } => {
                if let Some(details) = details {
                    self.show_message(&format!("{}: {}", message, details), EventSeverity::Error);
                } else {
                    self.show_message(&message, EventSeverity::Error);
                }
            }

            // Warning events
            Event::Warning { message, context } => {
                if let Some(context) = context {
                    self.show_message(
                        &format!("{} ({})", message, context),
                        EventSeverity::Warning,
                    );
                } else {
                    self.show_message(&message, EventSeverity::Warning);
                }
            }

            // Quality Assurance events
            Event::QaCheckStarted {
                check_type,
                check_name,
            } => {
                self.show_operation_message(
                    &format!("Starting {} check: {}", check_type, check_name),
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
                        &format!("{} check passed: {}", check_type, check_name),
                        "qa",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!(
                            "{} check completed: {} ({} findings)",
                            check_type, check_name, findings_count
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
                    &format!("{} check failed: {} - {}", check_type, check_name, error),
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
                    &format!(
                        "Starting QA pipeline for {} {} (level: {})",
                        package, version, qa_level
                    ),
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
                            "QA pipeline completed for {} {}: {}/{} checks passed ({}s)",
                            package, version, passed, total_checks, duration_seconds
                        ),
                        "qa",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!(
                            "QA pipeline completed for {} {}: {}/{} passed, {} failed ({}s)",
                            package, version, passed, total_checks, failed, duration_seconds
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
                    (Some(path), Some(line)) => format!(" ({}:{})", path, line),
                    (Some(path), None) => format!(" ({})", path),
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
                        &format!("QA report generated: {} ({})", path, format),
                        "qa",
                        EventSeverity::Success,
                    );
                } else {
                    self.show_operation_message(
                        &format!("QA report generated ({})", format),
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
                            .map(|(k, v)| format!("{}={}", k, v))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.show_message(
                            &format!("{} ({})", message, context_str),
                            EventSeverity::Debug,
                        );
                    }
                }
            }

            // Catch-all for other events (silently ignore for now)
            _ => {
                // These events are not displayed in the CLI
                // but could be logged if debug mode is enabled
                if self.debug_enabled {
                    self.show_message(
                        &format!("Unhandled event: {:?}", event),
                        EventSeverity::Debug,
                    );
                }
            }
        }
    }

    /// Handle download started event
    fn handle_download_started(&mut self, url: &str, size: Option<u64>) {
        let filename = url.split('/').next_back().unwrap_or(url);

        let pb = if let Some(total) = size {
            ProgressBar::new(total)
        } else {
            ProgressBar::new_spinner()
        };

        // Enhanced progress bar styling
        let template = if self.ui_style.colors_enabled
            && self.ui_style.term.features().colors_supported()
        {
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) {msg}"
        } else {
            "{spinner} [{elapsed_precise}] [{wide_bar}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) {msg}"
        };

        pb.set_style(
            ProgressStyle::default_bar()
                .template(template)
                .unwrap()
                .progress_chars("━━╾─"),
        );

        let icon = self.ui_style.get_operation_icon("download");
        pb.set_message(format!("{} {}", icon, filename));

        let pb = self.multi_progress.add(pb);
        self.download_bars.insert(url.to_string(), pb);
    }

    /// Handle download progress event
    fn handle_download_progress(&mut self, url: &str, bytes_downloaded: u64, total_bytes: u64) {
        if let Some(pb) = self.download_bars.get(url) {
            pb.set_length(total_bytes);
            pb.set_position(bytes_downloaded);
        }
    }

    /// Handle download completed event
    fn handle_download_completed(&mut self, url: &str) {
        if let Some(pb) = self.download_bars.remove(url) {
            let icon = if self.ui_style.colors_enabled
                && self.ui_style.term.features().colors_supported()
            {
                "✅"
            } else {
                "✓"
            };
            pb.finish_with_message(format!("{} Downloaded", icon));
        }
    }

    /// Handle download failed event
    fn handle_download_failed(&mut self, url: &str, error: &str) {
        if let Some(pb) = self.download_bars.remove(url) {
            let icon = if self.ui_style.colors_enabled
                && self.ui_style.term.features().colors_supported()
            {
                "❌"
            } else {
                "✗"
            };
            pb.finish_with_message(format!("{} Failed: {}", icon, error));
        }
    }

    /// Handle installation started event - create loading animation
    fn handle_install_started(&mut self, package_name: &str) {
        let pb = ProgressBar::new_spinner();

        // Enhanced spinner styling
        let template =
            if self.ui_style.colors_enabled && self.ui_style.term.features().colors_supported() {
                "{spinner:.green} {msg}"
            } else {
                "{spinner} {msg}"
            };

        pb.set_style(
            ProgressStyle::default_spinner()
                .template(template)
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );

        let icon = self.ui_style.get_operation_icon("install");
        let styled_message = self.ui_style.style_operation_message(
            &format!("Installing {}", package_name),
            "install",
            EventSeverity::Info,
        );
        pb.set_message(format!("{} {}", icon, styled_message));

        let pb = self.multi_progress.add(pb);
        self.install_bars.insert(package_name.to_string(), pb);
    }

    /// Handle installation completed event - finish animation
    fn handle_install_completed(&mut self, package_name: &str, state_id: &impl std::fmt::Display) {
        if let Some(pb) = self.install_bars.remove(package_name) {
            let icon = if self.ui_style.colors_enabled
                && self.ui_style.term.features().colors_supported()
            {
                "✅"
            } else {
                "✓"
            };
            let styled_message = self.ui_style.style_operation_message(
                &format!("Installed {} (state: {})", package_name, state_id),
                "install",
                EventSeverity::Success,
            );
            pb.finish_with_message(format!("{} {}", icon, styled_message));
        }
    }

    /// Handle installation failed event - finish animation with error
    fn handle_install_failed(&mut self, package_name: &str, error: &str) {
        if let Some(pb) = self.install_bars.remove(package_name) {
            let icon = if self.ui_style.colors_enabled
                && self.ui_style.term.features().colors_supported()
            {
                "❌"
            } else {
                "✗"
            };
            let styled_message = self.ui_style.style_operation_message(
                &format!("Failed to install {}: {}", package_name, error),
                "install",
                EventSeverity::Error,
            );
            pb.finish_with_message(format!("{} {}", icon, styled_message));
        }
    }

    /// Show styled message based on severity
    fn show_message(&self, message: &str, severity: EventSeverity) {
        let prefix = self.ui_style.get_prefix(severity);
        let styled_message = self.ui_style.style_message(message, severity);
        let formatted = format!("{} {}", prefix, styled_message);
        self.multi_progress.println(formatted).unwrap_or(());
    }

    /// Show operation message with appropriate icon and styling
    fn show_operation_message(&self, message: &str, operation: &str, severity: EventSeverity) {
        let icon = self.ui_style.get_operation_icon(operation);
        let styled_message = self
            .ui_style
            .style_operation_message(message, operation, severity);
        let formatted = format!("{} {}", icon, styled_message);
        self.multi_progress.println(formatted).unwrap_or(());
    }

    /// Show warning message
    fn show_warning(&self, message: &str) {
        // Use multi_progress to avoid interfering with progress bars
        self.multi_progress
            .println(format!("WARNING: {}", message))
            .unwrap_or(());
    }
}

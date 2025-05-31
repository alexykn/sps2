//! Event handling and progress display

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sps2_events::Event;
use std::collections::HashMap;

/// Event handler for progress display and user feedback
pub struct EventHandler {
    /// Multi-progress manager for concurrent progress bars
    multi_progress: MultiProgress,
    /// Active progress bars by URL
    download_bars: HashMap<String, ProgressBar>,
    /// Output renderer for final results
    #[allow(dead_code)]
    renderer: crate::display::OutputRenderer,
}

impl EventHandler {
    /// Create new event handler
    pub fn new(renderer: crate::display::OutputRenderer) -> Self {
        Self {
            multi_progress: MultiProgress::new(),
            download_bars: HashMap::new(),
            renderer,
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
                self.show_status(&format!("📦 Installing {} {}", name, version));
            }
            Event::PackageInstalled {
                name,
                version,
                path: _,
            } => {
                self.show_status(&format!("✅ Installed {} {}", name, version));
            }
            Event::PackageDownloaded { name, version } => {
                self.show_status(&format!("📥 Downloaded {} {}", name, version));
            }
            Event::PackageBuilding { name, version } => {
                self.show_status(&format!("🔨 Building {} {}", name, version));
            }

            // State events
            Event::StateCreating { state_id } => {
                self.show_status(&format!("🔄 Creating state {}", state_id));
            }
            Event::StateTransition {
                from,
                to,
                operation: _,
            } => {
                self.show_status(&format!("🔄 State transition {} → {}", from, to));
            }

            // Build events
            Event::BuildStarting { package, version } => {
                self.show_status(&format!("🔨 Starting build of {} {}", package, version));
            }
            Event::BuildCompleted {
                package,
                version,
                path,
            } => {
                self.show_status(&format!(
                    "✅ Built {} {} → {}",
                    package,
                    version,
                    path.display()
                ));
            }
            Event::BuildFailed {
                package,
                version,
                error,
            } => {
                self.show_error(&format!(
                    "❌ Build failed for {} {}: {}",
                    package, version, error
                ));
            }
            Event::BuildCommand { package, command } => {
                self.show_status(&format!("🔧 {} > {}", package, command));
            }
            Event::BuildCleaned { package } => {
                self.show_status(&format!("🧹 Cleaned build for {}", package));
            }

            // Resolver events
            Event::DependencyResolving { package, count } => {
                if count == 1 {
                    self.show_status(&format!("🔍 Resolving dependencies for {}", package));
                } else {
                    self.show_status(&format!("🔍 Resolving dependencies for {} packages", count));
                }
            }
            Event::DependencyResolved {
                package,
                version: _,
                count,
            } => {
                if count == 1 {
                    self.show_status(&format!("✅ Resolved dependencies for {}", package));
                } else {
                    self.show_status(&format!("✅ Resolved {} dependencies", count));
                }
            }

            // Operation events
            Event::InstallStarting { packages } => {
                if packages.len() == 1 {
                    self.show_status(&format!("📦 Installing {}", packages[0]));
                } else {
                    self.show_status(&format!("📦 Installing {} packages", packages.len()));
                }
            }
            Event::InstallCompleted { packages, state_id } => {
                if packages.len() == 1 {
                    self.show_status(&format!(
                        "✅ Installed {} (state: {})",
                        packages[0], state_id
                    ));
                } else {
                    self.show_status(&format!(
                        "✅ Installed {} packages (state: {})",
                        packages.len(),
                        state_id
                    ));
                }
            }
            Event::UninstallStarting { packages } => {
                if packages.len() == 1 {
                    self.show_status(&format!("🗑️  Uninstalling {}", packages[0]));
                } else {
                    self.show_status(&format!("🗑️  Uninstalling {} packages", packages.len()));
                }
            }
            Event::UninstallCompleted { packages, state_id } => {
                if packages.len() == 1 {
                    self.show_status(&format!(
                        "✅ Uninstalled {} (state: {})",
                        packages[0], state_id
                    ));
                } else {
                    self.show_status(&format!(
                        "✅ Uninstalled {} packages (state: {})",
                        packages.len(),
                        state_id
                    ));
                }
            }
            Event::UpdateStarting { packages } => {
                if packages.len() == 1 && packages[0] == "all" {
                    self.show_status("🔄 Updating all packages");
                } else if packages.len() == 1 {
                    self.show_status(&format!("🔄 Updating {}", packages[0]));
                } else {
                    self.show_status(&format!("🔄 Updating {} packages", packages.len()));
                }
            }
            Event::UpdateCompleted { packages, state_id } => {
                if packages.is_empty() {
                    self.show_status(&format!("ℹ️  No updates available (state: {})", state_id));
                } else if packages.len() == 1 {
                    self.show_status(&format!("✅ Updated {} (state: {})", packages[0], state_id));
                } else {
                    self.show_status(&format!(
                        "✅ Updated {} packages (state: {})",
                        packages.len(),
                        state_id
                    ));
                }
            }
            Event::UpgradeStarting { packages } => {
                if packages.len() == 1 && packages[0] == "all" {
                    self.show_status("⬆️  Upgrading all packages");
                } else if packages.len() == 1 {
                    self.show_status(&format!("⬆️  Upgrading {}", packages[0]));
                } else {
                    self.show_status(&format!("⬆️  Upgrading {} packages", packages.len()));
                }
            }
            Event::UpgradeCompleted { packages, state_id } => {
                if packages.is_empty() {
                    self.show_status(&format!("ℹ️  No upgrades available (state: {})", state_id));
                } else if packages.len() == 1 {
                    self.show_status(&format!(
                        "✅ Upgraded {} (state: {})",
                        packages[0], state_id
                    ));
                } else {
                    self.show_status(&format!(
                        "✅ Upgraded {} packages (state: {})",
                        packages.len(),
                        state_id
                    ));
                }
            }

            // Repository events
            Event::RepoSyncStarting => {
                self.show_status("🔄 Syncing repository index");
            }
            Event::RepoSyncCompleted {
                packages_updated,
                duration_ms,
            } => {
                if packages_updated == 0 {
                    self.show_status(&format!(
                        "ℹ️  Repository index up to date ({}ms)",
                        duration_ms
                    ));
                } else {
                    self.show_status(&format!(
                        "✅ Updated {} packages ({}ms)",
                        packages_updated, duration_ms
                    ));
                }
            }

            // Search events
            Event::SearchStarting { query } => {
                self.show_status(&format!("🔍 Searching for '{}'", query));
            }
            Event::SearchCompleted { query: _, count } => {
                self.show_status(&format!("✅ Found {} packages", count));
            }

            // List events
            Event::ListStarting => {
                self.show_status("📋 Listing installed packages");
            }
            Event::ListCompleted { count } => {
                self.show_status(&format!("✅ Found {} installed packages", count));
            }

            // Cleanup events
            Event::CleanupStarting => {
                self.show_status("🧹 Cleaning up system");
            }
            Event::CleanupCompleted {
                states_removed,
                packages_removed,
                duration_ms,
            } => {
                self.show_status(&format!(
                    "✅ Cleaned {} states and {} packages ({}ms)",
                    states_removed, packages_removed, duration_ms
                ));
            }

            // Rollback events
            Event::RollbackStarting { target_state } => {
                self.show_status(&format!("⏪ Rolling back to state {}", target_state));
            }
            Event::RollbackCompleted {
                target_state,
                duration_ms,
            } => {
                self.show_status(&format!(
                    "✅ Rolled back to {} ({}ms)",
                    target_state, duration_ms
                ));
            }

            // Health check events
            Event::HealthCheckStarting => {
                self.show_status("🔍 Checking system health");
            }
            Event::HealthCheckCompleted { healthy, issues } => {
                if healthy {
                    self.show_status("✅ System healthy");
                } else {
                    self.show_status(&format!("⚠️  {} issues found", issues.len()));
                }
            }

            // Operation events
            Event::OperationStarted { operation } => {
                self.show_status(&format!("🔄 {}", operation));
            }
            Event::OperationFailed { operation, error } => {
                self.show_error(&format!("❌ {} failed: {}", operation, error));
            }

            // Index events
            Event::IndexUpdateStarting { url } => {
                self.show_status(&format!("📥 Updating index from {}", url));
            }
            Event::IndexUpdateCompleted {
                packages_added,
                packages_updated,
            } => {
                self.show_status(&format!(
                    "✅ Index updated: {} added, {} updated",
                    packages_added, packages_updated
                ));
            }

            // State rollback event
            Event::StateRollback { from, to } => {
                self.show_status(&format!("⏪ Rolled back from {} to {}", from, to));
            }

            // Error events
            Event::Error { message, details } => {
                if let Some(details) = details {
                    self.show_error(&format!("❌ {}: {}", message, details));
                } else {
                    self.show_error(&format!("❌ {}", message));
                }
            }

            // Catch-all for other events (silently ignore for now)
            _ => {
                // These events are not displayed in the CLI
                // but could be logged if debug mode is enabled
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

        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} {msg}")
                .unwrap()
                .progress_chars("#>-")
        );

        pb.set_message(format!("Downloading {}", filename));

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
            pb.finish_with_message("Downloaded");
        }
    }

    /// Handle download failed event
    fn handle_download_failed(&mut self, url: &str, error: &str) {
        if let Some(pb) = self.download_bars.remove(url) {
            pb.finish_with_message(format!("Failed: {}", error));
        }
    }

    /// Show status message
    fn show_status(&self, message: &str) {
        // Use multi_progress to avoid interfering with progress bars
        self.multi_progress.println(message).unwrap_or(());
    }

    /// Show error message
    fn show_error(&self, message: &str) {
        // Use multi_progress to avoid interfering with progress bars
        self.multi_progress.println(message).unwrap_or(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_types::{ColorChoice, Version};

    #[test]
    fn test_event_handler_creation() {
        let renderer = crate::display::OutputRenderer::new(false, ColorChoice::Auto);
        let mut handler = EventHandler::new(renderer);

        // Test handling basic events
        handler.handle_event(Event::InstallStarting {
            packages: vec!["curl".to_string()],
        });

        handler.handle_event(Event::PackageInstalling {
            name: "curl".to_string(),
            version: Version::parse("8.5.0").unwrap(),
        });

        // Verify no panics occur
    }

    #[test]
    fn test_download_event_handling() {
        let renderer = crate::display::OutputRenderer::new(false, ColorChoice::Auto);
        let mut handler = EventHandler::new(renderer);

        let url = "https://example.com/package.sp";

        // Start download
        handler.handle_event(Event::DownloadStarted {
            url: url.to_string(),
            size: Some(1024),
        });

        // Check that progress bar was created
        assert!(handler.download_bars.contains_key(url));

        // Progress update
        handler.handle_event(Event::DownloadProgress {
            url: url.to_string(),
            bytes_downloaded: 512,
            total_bytes: 1024,
        });

        // Complete download
        handler.handle_event(Event::DownloadCompleted {
            url: url.to_string(),
            size: 1024,
        });

        // Check that progress bar was removed
        assert!(!handler.download_bars.contains_key(url));
    }
}

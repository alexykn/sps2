#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Event system for async communication in spsv2
//!
//! This crate provides the event types and channel aliases used for
//! communication between crates. All output goes through events - no
//! direct logging or printing is allowed outside the CLI.

use serde::{Deserialize, Serialize};
use spsv2_types::{StateId, Version};
use std::collections::HashMap;

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
    PackageDownloaded {
        name: String,
        version: Version,
    },
    PackageInstalled {
        name: String,
        version: Version,
        path: String,
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
}

/// Health status for components
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Ok,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_channel() {
        let (tx, mut rx) = channel();

        tx.emit(Event::warning("test warning"));

        let event = rx.recv().await.unwrap();
        match event {
            Event::Warning { message, .. } => {
                assert_eq!(message, "test warning");
            }
            _ => panic!("Wrong event type"),
        }
    }

    #[test]
    fn test_event_serialization() {
        let event = Event::PackageInstalling {
            name: "jq".to_string(),
            version: Version::parse("1.7.0").unwrap(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();

        match deserialized {
            Event::PackageInstalling { name, version } => {
                assert_eq!(name, "jq");
                assert_eq!(version, Version::parse("1.7.0").unwrap());
            }
            _ => panic!("Wrong event type"),
        }
    }
}

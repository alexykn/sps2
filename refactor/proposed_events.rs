//! Proposed minimal event surface for sps2 after consolidation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::Level;
use uuid::Uuid;

/// Shared metadata carried by every event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMeta {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    pub issued_at: DateTime<Utc>,
    pub level: Level,
    pub source: &'static str,
    /// Correlation identifier ("install:pkg@ver") for stitching logs, UI, and telemetry.
    pub correlation: Option<String>,
}

/// High-level semantic milestones that matter to multiple subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomainEvent {
    ResolveStarted { spec: String },
    ResolveCompleted { packages: usize },
    ResolveFailed { code: &'static str },

    FetchStarted { pkg: String, bytes: Option<u64> },
    FetchCompleted { pkg: String, bytes: u64, checksum: String },
    FetchFailed { code: &'static str },

    InstallStarted { pkg: String, target: String },
    InstallCommitted { pkg: String, files: usize },
    InstallRolledBack { pkg: String, reason_code: &'static str },
}

/// Granular progress notifications; replace the current ProgressEvent mega-enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    Started { operation: String, total: Option<u64> },
    Advanced { current: u64, total: Option<u64>, phase: Phase },
    Completed { duration_ms: u64 },
    Failed { code: &'static str, details: Option<String> },
}

/// Optional progress phases used by UI to label sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Phase {
    Resolve,
    Fetch,
    Build,
    Install,
    Verify,
    Cleanup,
}

/// Diagnostic/telemetry events (structured logging). Consumers may choose to ignore.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagnosticEvent {
    Warning { code: &'static str, message: String },
    Info { message: String },
    Trace { message: String },
}

/// Unified application event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppEvent {
    Domain(EventMeta, DomainEvent),
    Progress(EventMeta, ProgressEvent),
    Diagnostic(EventMeta, DiagnosticEvent),
}

/// Simple emitter trait with explicit meta injection.
pub trait EventBus {
    fn emit(&self, event: AppEvent);
}

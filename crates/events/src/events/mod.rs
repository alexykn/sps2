use serde::{Deserialize, Serialize};

// Declare all domain modules
pub mod general;
pub mod download;
pub mod build;
pub mod state;
pub mod lifecycle;
pub mod progress;
pub mod repo;
pub mod guard;
pub mod qa;
pub mod audit;
pub mod python;
pub mod package;

// Re-export all domain events
pub use general::*;
pub use download::*;
pub use build::*;
pub use state::*;
pub use lifecycle::*;
pub use progress::*;
pub use repo::*;
pub use guard::*;
pub use qa::*;
pub use audit::*;
pub use python::*;
pub use package::*;

/// Top-level application event enum that aggregates all domain-specific events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "domain", content = "event", rename_all = "snake_case")]
pub enum AppEvent {
    /// General utility events (warnings, errors, operations)
    General(GeneralEvent),
    
    /// Download-specific events (HTTP downloads, progress, retries)
    Download(DownloadEvent),
    
    /// Build system events (compilation, caching, sessions)
    Build(BuildEvent),
    
    /// State management events (transactions, rollbacks)
    State(StateEvent),
    
    /// Package lifecycle events (install, uninstall, validation)
    Lifecycle(LifecycleEvent),
    
    /// Progress tracking events (sophisticated progress algorithms)
    Progress(ProgressEvent),
    
    /// Repository and index events (sync, mirroring)
    Repo(RepoEvent),
    
    /// Guard events (filesystem integrity, healing)
    Guard(GuardEvent),
    
    /// Quality assurance events (artifact validation)
    Qa(QaEvent),
    
    /// Audit and vulnerability scanning events
    Audit(AuditEvent),
    
    /// Python virtual environment events
    Python(PythonEvent),
    
    /// Package operation events (high-level package operations)
    Package(PackageEvent),
}

impl AppEvent {
    /// Determine the appropriate tracing log level for this event
    pub fn log_level(&self) -> tracing::Level {
        use tracing::Level;
        
        match self {
            // Error-level events
            AppEvent::General(GeneralEvent::Error { .. }) => Level::ERROR,
            AppEvent::Download(DownloadEvent::Failed { .. }) => Level::ERROR,
            AppEvent::Build(BuildEvent::Failed { .. }) => Level::ERROR,
            AppEvent::State(StateEvent::TransitionFailed { .. }) => Level::ERROR,
            AppEvent::State(StateEvent::RollbackFailed { .. }) => Level::ERROR,
            AppEvent::Lifecycle(LifecycleEvent::ValidationFailed { .. }) => Level::ERROR,
            AppEvent::Lifecycle(LifecycleEvent::InstallationFailed { .. }) => Level::ERROR,
            AppEvent::Progress(ProgressEvent::Failed { .. }) => Level::ERROR,
            AppEvent::Guard(GuardEvent::VerificationFailed { .. }) => Level::ERROR,
            AppEvent::Guard(GuardEvent::HealingFailed { .. }) => Level::ERROR,
            AppEvent::Qa(QaEvent::CheckFailed { .. }) => Level::ERROR,
            
            // Warning-level events
            AppEvent::General(GeneralEvent::Warning { .. }) => Level::WARN,
            AppEvent::Build(BuildEvent::Warning { .. }) => Level::WARN,
            AppEvent::Download(DownloadEvent::Stalled { .. }) => Level::WARN,
            AppEvent::Progress(ProgressEvent::Stalled { .. }) => Level::WARN,
            AppEvent::Progress(ProgressEvent::BottleneckDetected { .. }) => Level::WARN,
            
            // Info-level events (completions, starts)
            AppEvent::Download(DownloadEvent::Completed { .. }) => Level::INFO,
            AppEvent::Build(BuildEvent::Completed { .. }) => Level::INFO,
            AppEvent::State(StateEvent::TransitionCompleted { .. }) => Level::INFO,
            AppEvent::Lifecycle(LifecycleEvent::InstallationCompleted { .. }) => Level::INFO,
            AppEvent::Progress(ProgressEvent::Completed { .. }) => Level::INFO,
            AppEvent::Guard(GuardEvent::VerificationCompleted { .. }) => Level::INFO,
            AppEvent::Qa(QaEvent::PipelineCompleted { .. }) => Level::INFO,
            AppEvent::Audit(AuditEvent::Completed { .. }) => Level::INFO,
            
            // Debug-level events (progress updates, internal state)
            AppEvent::General(GeneralEvent::DebugLog { .. }) => Level::DEBUG,
            AppEvent::Download(DownloadEvent::Progress { .. }) => Level::DEBUG,
            AppEvent::Build(BuildEvent::StepOutput { .. }) => Level::DEBUG,
            AppEvent::Progress(ProgressEvent::Updated { .. }) => Level::DEBUG,
            AppEvent::Guard(GuardEvent::VerificationProgress { .. }) => Level::DEBUG,
            
            // Trace-level events (very detailed internal operations)
            AppEvent::Build(BuildEvent::ResourceUsage { .. }) => Level::TRACE,
            AppEvent::Progress(ProgressEvent::StatisticsUpdated { .. }) => Level::TRACE,
            
            // Default to INFO for most events
            _ => Level::INFO,
        }
    }

    /// Get the log target for this event (for structured logging)
    pub fn log_target(&self) -> &'static str {
        match self {
            AppEvent::General(_) => "sps2::events::general",
            AppEvent::Download(_) => "sps2::events::download",
            AppEvent::Build(_) => "sps2::events::build",
            AppEvent::State(_) => "sps2::events::state",
            AppEvent::Lifecycle(_) => "sps2::events::lifecycle",
            AppEvent::Progress(_) => "sps2::events::progress",
            AppEvent::Repo(_) => "sps2::events::repo",
            AppEvent::Guard(_) => "sps2::events::guard",
            AppEvent::Qa(_) => "sps2::events::qa",
            AppEvent::Audit(_) => "sps2::events::audit",
            AppEvent::Python(_) => "sps2::events::python",
            AppEvent::Package(_) => "sps2::events::package",
        }
    }

    /// Get structured fields for logging (simplified for now)
    pub fn log_fields(&self) -> String {
        // For now, use debug formatting. In the future, this could be more sophisticated
        // with structured key-value pairs extracted from each event type.
        format!("{:?}", self)
    }
}
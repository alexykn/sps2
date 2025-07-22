use serde::{Deserialize, Serialize};

// Declare all domain modules
pub mod acquisition;
pub mod audit;
pub mod build;
pub mod download;
pub mod general;
pub mod guard;
pub mod install;
pub mod package;
pub mod progress;
pub mod python;
pub mod qa;
pub mod repo;
pub mod resolver;
pub mod state;
pub mod uninstall;
pub mod update;

// Re-export all domain events
pub use acquisition::*;
pub use audit::*;
pub use build::*;
pub use download::*;
pub use general::*;
pub use guard::*;
pub use install::*;
pub use package::*;
pub use progress::*;
pub use python::*;
pub use qa::*;
pub use repo::*;
pub use resolver::*;
pub use state::*;
pub use uninstall::*;
pub use update::*;

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

    /// Package installation events (staging, installation, validation)
    Install(InstallEvent),

    /// Package uninstallation events (removal, dependency checking)
    Uninstall(UninstallEvent),

    /// Package update/upgrade events (update planning, batch updates)
    Update(UpdateEvent),

    /// Package acquisition events (download, cache, verification)
    Acquisition(AcquisitionEvent),

    /// Progress tracking events (sophisticated progress algorithms)
    Progress(ProgressEvent),

    /// Repository and index events (sync, mirroring)
    Repo(RepoEvent),

    /// Resolver events (dependency resolution, SAT solving)
    Resolver(ResolverEvent),

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
            AppEvent::Install(InstallEvent::ValidationFailed { .. }) => Level::ERROR,
            AppEvent::Install(InstallEvent::Failed { .. }) => Level::ERROR,
            AppEvent::Uninstall(UninstallEvent::Failed { .. }) => Level::ERROR,
            AppEvent::Update(UpdateEvent::Failed { .. }) => Level::ERROR,
            AppEvent::Acquisition(AcquisitionEvent::Failed { .. }) => Level::ERROR,
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
            AppEvent::Install(InstallEvent::Completed { .. }) => Level::INFO,
            AppEvent::Uninstall(UninstallEvent::Completed { .. }) => Level::INFO,
            AppEvent::Update(UpdateEvent::Completed { .. }) => Level::INFO,
            AppEvent::Acquisition(AcquisitionEvent::Completed { .. }) => Level::INFO,
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
            AppEvent::Install(_) => "sps2::events::install",
            AppEvent::Uninstall(_) => "sps2::events::uninstall",
            AppEvent::Update(_) => "sps2::events::update",
            AppEvent::Acquisition(_) => "sps2::events::acquisition",
            AppEvent::Progress(_) => "sps2::events::progress",
            AppEvent::Repo(_) => "sps2::events::repo",
            AppEvent::Resolver(_) => "sps2::events::resolver",
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

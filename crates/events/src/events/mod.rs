use serde::{Deserialize, Serialize};

use crate::EventSource;

// Declare all domain modules
pub mod acquisition;
pub mod audit;
pub mod build;
pub mod download;
pub mod general;
pub mod guard;
pub mod install;
pub mod package;
pub mod platform;
pub mod progress;
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
pub use platform::*;
pub use progress::*;
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

    /// Package operation events (high-level package operations)
    Package(PackageEvent),

    /// Platform-specific operation events (binary, filesystem, process operations)
    Platform(PlatformEvent),
}

impl AppEvent {
    /// Identify the source domain for this event (used for metadata/logging).
    #[must_use]
    pub fn event_source(&self) -> EventSource {
        match self {
            AppEvent::General(_) => EventSource::GENERAL,
            AppEvent::Download(_) => EventSource::DOWNLOAD,
            AppEvent::Build(_) => EventSource::BUILD,
            AppEvent::State(_) => EventSource::STATE,
            AppEvent::Install(_) => EventSource::INSTALL,
            AppEvent::Uninstall(_) => EventSource::UNINSTALL,
            AppEvent::Update(_) => EventSource::UPDATE,
            AppEvent::Acquisition(_) => EventSource::ACQUISITION,
            AppEvent::Progress(_) => EventSource::PROGRESS,
            AppEvent::Repo(_) => EventSource::REPO,
            AppEvent::Resolver(_) => EventSource::RESOLVER,
            AppEvent::Guard(_) => EventSource::GUARD,
            AppEvent::Qa(_) => EventSource::QA,
            AppEvent::Audit(_) => EventSource::AUDIT,
            AppEvent::Package(_) => EventSource::PACKAGE,
            AppEvent::Platform(_) => EventSource::PLATFORM,
        }
    }

    /// Determine the appropriate tracing log level for this event
    #[must_use]
    pub fn log_level(&self) -> tracing::Level {
        use tracing::Level;

        match self {
            // Error-level events
            AppEvent::General(GeneralEvent::Error { .. })
            | AppEvent::Download(DownloadEvent::Failed { .. })
            | AppEvent::Build(BuildEvent::Failed { .. })
            | AppEvent::State(StateEvent::TransitionFailed { .. })
            | AppEvent::Install(InstallEvent::Failed { .. })
            | AppEvent::Uninstall(UninstallEvent::Failed { .. })
            | AppEvent::Update(UpdateEvent::Failed { .. })
            | AppEvent::Acquisition(AcquisitionEvent::Failed { .. })
            | AppEvent::Progress(ProgressEvent::Failed { .. })
            | AppEvent::Guard(
                GuardEvent::VerificationFailed { .. } | GuardEvent::HealingFailed { .. },
            )
            | AppEvent::Qa(QaEvent::CheckFailed { .. })
            | AppEvent::Platform(
                PlatformEvent::BinaryOperationFailed { .. }
                | PlatformEvent::FilesystemOperationFailed { .. }
                | PlatformEvent::ProcessExecutionFailed { .. },
            ) => Level::ERROR,

            // Warning-level events
            AppEvent::General(GeneralEvent::Warning { .. })
            | AppEvent::Build(BuildEvent::Warning { .. })
            | AppEvent::Progress(
                ProgressEvent::Stalled { .. } | ProgressEvent::BottleneckDetected { .. },
            ) => Level::WARN,

            // Debug-level events (progress updates, internal state)
            AppEvent::General(GeneralEvent::DebugLog { .. })
            | AppEvent::Build(BuildEvent::StepOutput { .. })
            | AppEvent::Progress(ProgressEvent::Updated { .. })
            | AppEvent::Guard(GuardEvent::VerificationProgress { .. }) => Level::DEBUG,

            // Trace-level events (very detailed internal operations)
            AppEvent::Build(BuildEvent::ResourceUsage { .. })
            | AppEvent::Progress(ProgressEvent::StatisticsUpdated { .. }) => Level::TRACE,

            // Default to INFO for most events
            _ => Level::INFO,
        }
    }

    /// Get the log target for this event (for structured logging)
    #[must_use]
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
            AppEvent::Package(_) => "sps2::events::package",
            AppEvent::Platform(_) => "sps2::events::platform",
        }
    }

    /// Get structured fields for logging (simplified for now)
    #[must_use]
    pub fn log_fields(&self) -> String {
        // For now, use debug formatting. In the future, this could be more sophisticated
        // with structured key-value pairs extracted from each event type.
        format!("{self:?}")
    }
}

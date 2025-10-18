use serde::{Deserialize, Serialize};

use crate::EventSource;
use sps2_errors::UserFacingError;

/// Structured failure information shared across domains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureContext {
    /// Optional stable error code once taxonomy lands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Short user-facing message.
    pub message: String,
    /// Optional remediation hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Whether retrying the operation might succeed.
    pub retryable: bool,
}

impl FailureContext {
    /// Construct a new failure context.
    #[must_use]
    pub fn new(
        code: Option<impl Into<String>>,
        message: impl Into<String>,
        hint: Option<impl Into<String>>,
        retryable: bool,
    ) -> Self {
        Self {
            code: code.map(Into::into),
            message: message.into(),
            hint: hint.map(Into::into),
            retryable,
        }
    }

    /// Build failure context from a `UserFacingError` implementation.
    #[must_use]
    pub fn from_error<E: UserFacingError + ?Sized>(error: &E) -> Self {
        Self::new(
            error.user_code(),
            error.user_message().into_owned(),
            error.user_hint(),
            error.is_retryable(),
        )
    }
}

// Declare all domain modules
pub mod build;
pub mod general;
pub mod guard;
pub mod lifecycle; // Generic lifecycle events (replaces acquisition, download, install, resolver, repo, uninstall, update)
pub mod package;
pub mod platform;
pub mod progress;
pub mod qa;
pub mod state;

// Re-export all domain events
pub use build::*;
pub use general::*;
pub use guard::*;
pub use lifecycle::*; // Generic lifecycle events (replaces old event types)
pub use package::*;
pub use platform::*;
pub use progress::*;
pub use qa::*;
pub use state::*;

/// Top-level application event enum that aggregates all domain-specific events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "domain", content = "event", rename_all = "snake_case")]
pub enum AppEvent {
    /// General utility events (warnings, errors, operations)
    General(GeneralEvent),

    /// Build system events (compilation, caching, sessions)
    Build(BuildEvent),

    /// State management events (transactions, rollbacks)
    State(StateEvent),

    /// Progress tracking events (sophisticated progress algorithms)
    Progress(ProgressEvent),

    /// Guard events (filesystem integrity, healing)
    Guard(GuardEvent),

    /// Quality assurance events (artifact validation)
    Qa(QaEvent),

    /// Package operation events (high-level package operations)
    Package(PackageEvent),

    /// Platform-specific operation events (binary, filesystem, process operations)
    Platform(PlatformEvent),

    /// Generic lifecycle events (acquisition, download, install, resolver, repo, uninstall, update)
    Lifecycle(LifecycleEvent),
}

impl AppEvent {
    /// Identify the source domain for this event (used for metadata/logging).
    #[must_use]
    pub fn event_source(&self) -> EventSource {
        match self {
            Self::General(_) => EventSource::GENERAL,
            Self::Build(_) => EventSource::BUILD,
            Self::State(_) => EventSource::STATE,
            Self::Progress(_) => EventSource::PROGRESS,
            Self::Guard(_) => EventSource::GUARD,
            Self::Qa(_) => EventSource::QA,
            Self::Package(_) => EventSource::PACKAGE,
            Self::Platform(_) => EventSource::PLATFORM,
            Self::Lifecycle(event) => match event.domain() {
                LifecycleDomain::Acquisition => EventSource::ACQUISITION,
                LifecycleDomain::Download => EventSource::DOWNLOAD,
                LifecycleDomain::Install => EventSource::INSTALL,
                LifecycleDomain::Resolver => EventSource::RESOLVER,
                LifecycleDomain::Repo => EventSource::REPO,
                LifecycleDomain::Uninstall => EventSource::UNINSTALL,
                LifecycleDomain::Update => EventSource::UPDATE,
            },
        }
    }

    /// Determine the appropriate tracing log level for this event
    #[must_use]
    pub fn log_level(&self) -> tracing::Level {
        use tracing::Level;

        match self {
            // Error-level events
            Self::General(GeneralEvent::Error { .. })
            | Self::Build(BuildEvent::Failed { .. })
            | Self::Progress(ProgressEvent::Failed { .. })
            | Self::Qa(QaEvent::PipelineFailed { .. })
            | Self::Package(PackageEvent::OperationFailed { .. })
            | Self::Platform(PlatformEvent::OperationFailed { .. })
            | Self::Guard(
                GuardEvent::VerificationFailed { .. } | GuardEvent::HealingFailed { .. },
            )
            | Self::State(
                StateEvent::TransitionFailed { .. }
                | StateEvent::RollbackFailed { .. }
                | StateEvent::CleanupFailed { .. },
            ) => Level::ERROR,

            // Lifecycle events - check stage
            Self::Lifecycle(event) if event.stage() == &LifecycleStage::Failed => Level::ERROR,

            // Warning-level events
            Self::General(GeneralEvent::Warning { .. })
            | Self::Build(BuildEvent::Diagnostic(build::BuildDiagnostic::Warning { .. })) => {
                Level::WARN
            }

            // Debug-level events (progress updates, internal state)
            Self::General(GeneralEvent::DebugLog { .. })
            | Self::Build(BuildEvent::Diagnostic(build::BuildDiagnostic::LogChunk {
                ..
            }))
            | Self::Progress(ProgressEvent::Updated { .. })
            | Self::Qa(QaEvent::CheckEvaluated { .. }) => Level::DEBUG,

            // Trace-level events (very detailed internal operations)
            Self::Build(BuildEvent::Diagnostic(build::BuildDiagnostic::CachePruned {
                ..
            })) => Level::TRACE,

            // Default to INFO for most events
            _ => Level::INFO,
        }
    }

    /// Get the log target for this event (for structured logging)
    #[must_use]
    pub fn log_target(&self) -> &'static str {
        match self {
            Self::General(_) => "sps2::events::general",
            Self::Build(_) => "sps2::events::build",
            Self::State(_) => "sps2::events::state",
            Self::Progress(_) => "sps2::events::progress",
            Self::Guard(_) => "sps2::events::guard",
            Self::Qa(_) => "sps2::events::qa",
            Self::Package(_) => "sps2::events::package",
            Self::Platform(_) => "sps2::events::platform",
            Self::Lifecycle(event) => match event.domain() {
                LifecycleDomain::Acquisition => "sps2::events::acquisition",
                LifecycleDomain::Download => "sps2::events::download",
                LifecycleDomain::Install => "sps2::events::install",
                LifecycleDomain::Resolver => "sps2::events::resolver",
                LifecycleDomain::Repo => "sps2::events::repo",
                LifecycleDomain::Uninstall => "sps2::events::uninstall",
                LifecycleDomain::Update => "sps2::events::update",
            },
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

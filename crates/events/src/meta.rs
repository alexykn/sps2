use std::borrow::Cow;
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::Level;
use uuid::Uuid;

/// Structured metadata that accompanies every event emission.
///
/// This wrapper gives consumers enough context to correlate events across
/// domains, attach them to tracing spans, and provide stable identifiers for
/// telemetry pipelines.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventMeta {
    /// Unique identifier for this specific event.
    pub event_id: Uuid,
    /// Parent event (when modelling hierarchical operations / progress).
    pub parent_id: Option<Uuid>,
    /// High-level correlation identifier (operation id, package key, etc.).
    pub correlation_id: Option<String>,
    /// Timestamp captured at emission time.
    pub timestamp: DateTime<Utc>,
    /// Severity used for routing to logging systems and alerting.
    pub level: EventLevel,
    /// Subsystem/component that originated the event.
    pub source: EventSource,
    /// Optional free-form labels for downstream enrichment (kept small on purpose).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
}

impl EventMeta {
    /// Create a new metadata instance for a given source and level.
    #[must_use]
    pub fn new(level: impl Into<EventLevel>, source: impl Into<EventSource>) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            parent_id: None,
            correlation_id: None,
            timestamp: Utc::now(),
            level: level.into(),
            source: source.into(),
            labels: BTreeMap::new(),
        }
    }

    /// Attach a correlation identifier used to stitch related events.
    #[must_use]
    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    /// Attach the parent event identifier for hierarchical operations.
    #[must_use]
    pub fn with_parent(mut self, parent_id: Uuid) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    /// Add an arbitrary label entry (kept intentionally small).
    #[must_use]
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }

    /// Convert the metadata level into a tracing level for downstream logging.
    #[must_use]
    pub fn tracing_level(&self) -> Level {
        self.level.into()
    }
}

/// Lightweight severity levels used by the event system.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<EventLevel> for Level {
    fn from(level: EventLevel) -> Self {
        match level {
            EventLevel::Trace => Level::TRACE,
            EventLevel::Debug => Level::DEBUG,
            EventLevel::Info => Level::INFO,
            EventLevel::Warn => Level::WARN,
            EventLevel::Error => Level::ERROR,
        }
    }
}

impl From<Level> for EventLevel {
    fn from(level: Level) -> Self {
        match level {
            Level::TRACE => EventLevel::Trace,
            Level::DEBUG => EventLevel::Debug,
            Level::INFO => EventLevel::Info,
            Level::WARN => EventLevel::Warn,
            Level::ERROR => EventLevel::Error,
        }
    }
}

/// Component/feature that originated the event.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub struct EventSource(Cow<'static, str>);

impl EventSource {
    pub const GENERAL: Self = Self::const_str("general");
    pub const DOWNLOAD: Self = Self::const_str("download");
    pub const BUILD: Self = Self::const_str("build");
    pub const STATE: Self = Self::const_str("state");
    pub const INSTALL: Self = Self::const_str("install");
    pub const UNINSTALL: Self = Self::const_str("uninstall");
    pub const UPDATE: Self = Self::const_str("update");
    pub const ACQUISITION: Self = Self::const_str("acquisition");
    pub const PROGRESS: Self = Self::const_str("progress");
    pub const REPO: Self = Self::const_str("repo");
    pub const RESOLVER: Self = Self::const_str("resolver");
    pub const GUARD: Self = Self::const_str("guard");
    pub const QA: Self = Self::const_str("qa");
    pub const AUDIT: Self = Self::const_str("audit");
    pub const PYTHON: Self = Self::const_str("python");
    pub const PACKAGE: Self = Self::const_str("package");
    pub const PLATFORM: Self = Self::const_str("platform");

    const fn const_str(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }

    /// Create a source value from any stringy input (e.g. crate path).
    #[must_use]
    pub fn from_dynamic(value: impl Into<String>) -> Self {
        let value = value.into();
        Self(Cow::Owned(value))
    }

    /// Borrow the underlying identifier used for logging/telemetry.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for EventSource {
    fn from(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }
}

impl From<String> for EventSource {
    fn from(value: String) -> Self {
        Self(Cow::Owned(value))
    }
}

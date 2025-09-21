#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Event system for async communication in sps2
//!
//! This crate provides a domain-driven event system with sophisticated
//! progress tracking, tracing integration, and clean separation of concerns.
//! All output goes through events - no direct logging or printing is allowed
//! outside the CLI.
//!
//! ## Architecture
//!
//! - **Domain-driven events**: Events grouped by functional domain (Build, Download, etc.)
//! - **Unified `EventEmitter` trait**: Single, consistent API for all event emissions
//! - **Tracing integration**: Built-in structured logging with intelligent log levels
//! - **Progress tracking**: Sophisticated algorithms with ETA, speed calculation, and phases

use serde::{Deserialize, Serialize};

pub mod meta;
pub use meta::{EventLevel, EventMeta, EventSource};

// Re-export the progress tracking system
pub mod progress;
pub use progress::*;

// Import the new domain-driven event system
pub mod events;
pub use events::{
    // Domain event types
    AcquisitionEvent,
    AppEvent,
    AuditEvent,
    BuildEvent,
    BuildSystem,
    DownloadEvent,
    FailureContext,
    GeneralEvent,
    GuardEvent,
    // Support types that don't conflict
    InstallEvent,
    PackageEvent,
    ProgressEvent,
    QaEvent,
    RepoEvent,
    ResolverEvent,
    StateEvent,
    UninstallEvent,
    UpdateEvent,
};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Envelope carrying metadata alongside an application event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventMessage {
    pub meta: EventMeta,
    pub event: AppEvent,
}

impl EventMessage {
    #[must_use]
    pub fn new(meta: EventMeta, event: AppEvent) -> Self {
        Self { meta, event }
    }

    #[must_use]
    pub fn from_event(event: AppEvent) -> Self {
        let meta = derive_meta(&event);
        Self { meta, event }
    }
}

/// Type alias for event sender using the new `AppEvent` system
pub type EventSender = UnboundedSender<EventMessage>;

/// Type alias for event receiver using the new `AppEvent` system
pub type EventReceiver = UnboundedReceiver<EventMessage>;

/// Create a new event channel with the `AppEvent` system
#[must_use]
pub fn channel() -> (EventSender, EventReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// The unified trait for emitting events throughout the sps2 system
///
/// This trait provides a single, consistent API for emitting events regardless of
/// whether you have a raw `EventSender` or a struct that contains one.
pub trait EventEmitter {
    /// Get the event sender for this emitter
    fn event_sender(&self) -> Option<&EventSender>;

    /// Allow implementers to enrich event metadata before emission.
    fn enrich_event_meta(&self, _event: &AppEvent, _meta: &mut EventMeta) {}

    /// Emit an event with explicitly provided metadata.
    fn emit_with_meta(&self, meta: EventMeta, event: AppEvent) {
        if let Some(sender) = self.event_sender() {
            let message = EventMessage::new(meta, event);
            let _ = sender.send(message);
        }
    }

    /// Emit an event through this emitter, automatically deriving metadata.
    fn emit(&self, event: AppEvent) {
        let mut meta = derive_meta(&event);
        self.enrich_event_meta(&event, &mut meta);
        self.emit_with_meta(meta, event);
    }

    /// Emit a debug log event
    fn emit_debug(&self, message: impl Into<String>) {
        self.emit(AppEvent::General(GeneralEvent::debug(message)));
    }

    /// Emit a debug log event with context
    fn emit_debug_with_context(
        &self,
        message: impl Into<String>,
        context: std::collections::HashMap<String, String>,
    ) {
        self.emit(AppEvent::General(GeneralEvent::debug_with_context(
            message, context,
        )));
    }

    /// Emit a warning event
    fn emit_warning(&self, message: impl Into<String>) {
        self.emit(AppEvent::General(GeneralEvent::warning(message)));
    }

    /// Emit a warning event with context
    fn emit_warning_with_context(&self, message: impl Into<String>, context: impl Into<String>) {
        self.emit(AppEvent::General(GeneralEvent::warning_with_context(
            message, context,
        )));
    }

    /// Emit an error event
    fn emit_error(&self, message: impl Into<String>) {
        self.emit(AppEvent::General(GeneralEvent::error(message)));
    }

    /// Emit an error event with details
    fn emit_error_with_details(&self, message: impl Into<String>, details: impl Into<String>) {
        self.emit(AppEvent::General(GeneralEvent::error_with_details(
            message, details,
        )));
    }

    /// Emit an operation started event
    fn emit_operation_started(&self, operation: impl Into<String>) {
        self.emit(AppEvent::General(GeneralEvent::OperationStarted {
            operation: operation.into(),
        }));
    }

    /// Emit an operation completed event
    fn emit_operation_completed(&self, operation: impl Into<String>, success: bool) {
        self.emit(AppEvent::General(GeneralEvent::OperationCompleted {
            operation: operation.into(),
            success,
        }));
    }

    /// Emit an operation failed event
    fn emit_operation_failed(&self, operation: impl Into<String>, failure: events::FailureContext) {
        self.emit(AppEvent::General(GeneralEvent::operation_failed(
            operation, failure,
        )));
    }

    /// Emit a download started event
    fn emit_download_started(
        &self,
        url: impl Into<String>,
        package: Option<String>,
        total_bytes: Option<u64>,
    ) {
        self.emit(AppEvent::Download(DownloadEvent::Started {
            url: url.into(),
            package,
            total_bytes,
        }));
    }

    /// Emit a download completed event
    fn emit_download_completed(
        &self,
        url: impl Into<String>,
        package: Option<String>,
        bytes_downloaded: u64,
    ) {
        self.emit(AppEvent::Download(DownloadEvent::Completed {
            url: url.into(),
            package,
            bytes_downloaded,
        }));
    }

    /// Emit a build started event
    fn emit_build_started(
        &self,
        session_id: impl Into<String>,
        package: impl Into<String>,
        version: sps2_types::Version,
    ) {
        self.emit(AppEvent::Build(BuildEvent::SessionStarted {
            session_id: session_id.into(),
            package: package.into(),
            version,
            build_system: BuildSystem::Custom,
            cache_enabled: false,
        }));
    }

    /// Emit a build completed event
    fn emit_build_completed(
        &self,
        session_id: impl Into<String>,
        package: impl Into<String>,
        version: sps2_types::Version,
        path: std::path::PathBuf,
    ) {
        self.emit(AppEvent::Build(BuildEvent::Completed {
            session_id: session_id.into(),
            package: package.into(),
            version,
            path,
            duration: std::time::Duration::from_secs(0),
        }));
    }

    /// Emit a progress started event
    fn emit_progress_started(
        &self,
        id: impl Into<String>,
        operation: impl Into<String>,
        total: Option<u64>,
    ) {
        self.emit(AppEvent::Progress(ProgressEvent::started(
            id, operation, total,
        )));
    }

    /// Emit a progress update event  
    fn emit_progress_updated(&self, id: impl Into<String>, current: u64, total: Option<u64>) {
        self.emit(AppEvent::Progress(ProgressEvent::updated(
            id, current, total,
        )));
    }

    /// Emit a progress completed event
    fn emit_progress_completed(&self, id: impl Into<String>, duration: std::time::Duration) {
        self.emit(AppEvent::Progress(ProgressEvent::completed(id, duration)));
    }

    /// Emit a progress failed event
    fn emit_progress_failed(&self, id: impl Into<String>, failure: events::FailureContext) {
        self.emit(AppEvent::Progress(ProgressEvent::failed(id, failure)));
    }
}

/// Implementation of `EventEmitter` for the raw `EventSender`
/// This allows `EventSender` to be used directly where `EventEmitter` is expected
impl EventEmitter for EventSender {
    fn event_sender(&self) -> Option<&EventSender> {
        Some(self)
    }
}

fn derive_meta(event: &AppEvent) -> EventMeta {
    let level: EventLevel = event.log_level().into();
    let source = event.event_source();
    EventMeta::new(level, source)
}

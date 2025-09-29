//! Execution context for parallel operations

use crate::SecurityPolicy;
use sps2_events::{EventEmitter, EventSender};

/// Execution context for parallel operations
#[derive(Clone)]
pub struct ExecutionContext {
    /// Event sender for progress reporting
    event_sender: Option<EventSender>,
    /// Optional security policy for signature enforcement
    security_policy: Option<SecurityPolicy>,
    /// Whether downloads should bypass cache reuse
    force_redownload: bool,
}

impl ExecutionContext {
    /// Create new execution context
    #[must_use]
    pub fn new() -> Self {
        Self {
            event_sender: None,
            security_policy: None,
            force_redownload: false,
        }
    }

    /// Set event sender
    #[must_use]
    pub fn with_event_sender(mut self, event_sender: EventSender) -> Self {
        self.event_sender = Some(event_sender);
        self
    }

    /// Set security policy for downloads
    #[must_use]
    pub fn with_security_policy(mut self, policy: SecurityPolicy) -> Self {
        self.security_policy = Some(policy);
        self
    }

    /// Set whether downloads must ignore cached packages
    #[must_use]
    pub fn with_force_redownload(mut self, force: bool) -> Self {
        self.force_redownload = force;
        self
    }

    /// Should downstream logic bypass store reuse
    pub fn force_redownload(&self) -> bool {
        self.force_redownload
    }

    /// Get the security policy if set
    pub(crate) fn security_policy(&self) -> Option<SecurityPolicy> {
        self.security_policy
    }
}

impl EventEmitter for ExecutionContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self::new()
    }
}
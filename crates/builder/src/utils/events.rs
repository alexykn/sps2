//! Event emission utilities for build operations

use crate::BuildContext;
use sps2_events::{Event, EventEmitter};

/// Send event if context has event sender
pub fn send_event(context: &BuildContext, event: Event) {
    context.emit_event(event);
}

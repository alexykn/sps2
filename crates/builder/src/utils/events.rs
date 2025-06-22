//! Event emission utilities for build operations

use crate::BuildContext;
use sps2_events::Event;

/// Send event if context has event sender
pub fn send_event(context: &BuildContext, event: Event) {
    if let Some(sender) = &context.event_sender {
        let _ = sender.send(event);
    }
}

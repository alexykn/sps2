//! Integration tests for events

#[cfg(test)]
mod tests {
    use spsv2_events::*;
    use spsv2_types::Version;

    #[tokio::test]
    async fn test_event_sender_ext() {
        let (tx, mut rx) = channel();

        // Test emit helper
        tx.emit(Event::error("test error"));
        tx.emit(Event::debug("test debug"));

        let event1 = rx.recv().await.unwrap();
        assert!(matches!(event1, Event::Error { .. }));

        let event2 = rx.recv().await.unwrap();
        assert!(matches!(event2, Event::DebugLog { .. }));
    }

    #[tokio::test]
    async fn test_dropped_receiver() {
        let (tx, rx) = channel();
        drop(rx);

        // Should not panic when receiver is dropped
        tx.emit(Event::warning("ignored"));
    }

    #[test]
    fn test_health_status_serialization() {
        let status = HealthStatus::Warning;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""warning""#);
    }
}

//! Example usage of the monitoring system

#[cfg(test)]
mod example {
    use crate::monitoring::{MonitoringConfig, MonitoringSystem};
    use sps2_types::Version;
    use std::time::Duration;
    
    /// Example: Using monitoring in a build operation
    #[tokio::test]
    async fn example_build_with_monitoring() {
        // Create event channel
        let (tx, mut rx) = sps2_events::channel();
        
        // Configure monitoring
        let monitoring_config = MonitoringConfig {
            enabled: true,
            level: crate::monitoring::MonitoringLevel::Standard,
            telemetry_interval: Duration::from_secs(2),
            aggregation_interval: Duration::from_secs(30),
            ..Default::default()
        };
        
        // Create monitoring system
        let monitoring = MonitoringSystem::new(monitoring_config, tx.clone())
            .await
            .unwrap();
        
        // Start monitoring for a build
        let build_monitor = monitoring
            .start_build("example-package", &Version::new(1, 0, 0))
            .await
            .unwrap();
        
        // Record build steps
        build_monitor.record_step("configure").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        build_monitor.record_step("compile").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // Record resource usage
        build_monitor.record_resources().await.unwrap();
        
        build_monitor.record_step("test").await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Complete the build
        build_monitor.complete(true).await.unwrap();
        
        // Get metrics snapshot
        let snapshot = monitoring.get_metrics_snapshot().await.unwrap();
        println!("Collected {} metrics", snapshot.metrics.len());
        
        // Get aggregated statistics
        let stats = monitoring.get_statistics().await.unwrap();
        println!("Build statistics: {:?}", stats.builds);
        
        // Export metrics (example)
        let prometheus_data = monitoring
            .export_metrics(crate::monitoring::ExportFormat::Prometheus)
            .await
            .unwrap();
        println!("Prometheus export size: {} bytes", prometheus_data.len());
        
        // Shutdown monitoring
        monitoring.shutdown().await.unwrap();
        
        // Check events
        let mut event_count = 0;
        while let Ok(event) = rx.try_recv() {
            event_count += 1;
            match event {
                sps2_events::Event::BuildCompleted { .. } => {
                    println!("Build completed event received");
                }
                sps2_events::Event::DebugLog { message, .. } => {
                    println!("Debug: {}", message);
                }
                _ => {}
            }
        }
        println!("Received {} events", event_count);
    }
    
    /// Example: Production configuration
    #[test]
    fn example_production_config() {
        use crate::config::BuildConfig;
        
        // Create build config with production monitoring
        let config = BuildConfig::default()
            .with_prod_monitoring()
            .with_monitoring_config(
                MonitoringConfig::production()
                    .with_database("/var/sps2/monitoring.db".into())
                    .with_export(crate::monitoring::config::ExportConfig {
                        prometheus: true,
                        prometheus_endpoint: Some("0.0.0.0:9090".to_string()),
                        ..Default::default()
                    })
            );
        
        assert!(config.monitoring_config.enabled);
        assert!(config.monitoring_config.database_path.is_some());
    }
}
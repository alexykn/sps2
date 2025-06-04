// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Monitoring and observability infrastructure for package builds
//!
//! This module provides comprehensive monitoring capabilities including:
//! - Telemetry collection for build metrics and resource usage
//! - Metrics aggregation with time-series data
//! - Distributed tracing with OpenTelemetry
//! - Real-time monitoring pipeline
//!
//! All monitoring is optional and can be disabled via `BuildConfig`.

pub mod aggregator;
pub mod config;
pub mod metrics;
pub mod pipeline;
pub mod telemetry;
pub mod tracing;

pub use aggregator::{MetricsAggregator, StatisticalSummary};
pub use config::{MonitoringConfig, MonitoringLevel};
pub use metrics::{Metric, MetricType, MetricsCollector, MetricsSnapshot};
pub use pipeline::{MonitoringPipeline, PipelineConfig};
pub use telemetry::{ResourceMetrics, SpanContext, TelemetryCollector};
pub use tracing::{BuildSpan, TracingCollector};

use sps2_errors::Error;
use sps2_events::{Event, EventSender};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Main monitoring coordinator
#[derive(Clone)]
pub struct MonitoringSystem {
    config: Arc<MonitoringConfig>,
    telemetry: Arc<TelemetryCollector>,
    metrics: Arc<MetricsCollector>,
    aggregator: Arc<RwLock<MetricsAggregator>>,
    tracing: Arc<TracingCollector>,
    pipeline: Arc<MonitoringPipeline>,
    tx: EventSender,
}

impl MonitoringSystem {
    /// Create a new monitoring system
    pub async fn new(config: MonitoringConfig, tx: EventSender) -> Result<Self, Error> {
        let config = Arc::new(config);

        let telemetry = Arc::new(TelemetryCollector::new(config.clone())?);
        let metrics = Arc::new(MetricsCollector::new(config.clone())?);
        let aggregator = Arc::new(RwLock::new(MetricsAggregator::new(config.clone())));
        let tracing = Arc::new(TracingCollector::new(config.clone())?);

        let pipeline_config = PipelineConfig::from_monitoring_config(&config);
        let pipeline = Arc::new(
            MonitoringPipeline::new(
                pipeline_config,
                metrics.clone(),
                aggregator.clone(),
                tx.clone(),
            )
            .await?,
        );

        Ok(Self {
            config,
            telemetry,
            metrics,
            aggregator,
            tracing,
            pipeline,
            tx,
        })
    }

    /// Start monitoring for a new build
    pub async fn start_build(
        &self,
        package_name: &str,
        version: &sps2_types::Version,
    ) -> Result<BuildMonitor, Error> {
        if !self.config.enabled {
            return Ok(BuildMonitor::disabled());
        }

        // Create a new trace span for this build
        let span = self
            .tracing
            .create_build_span(package_name, version)
            .await?;

        // Start telemetry collection
        self.telemetry.start_collection(&span.context).await?;

        // Initialize metrics for this build
        self.metrics.init_build_metrics(&span.context).await?;

        // Start the monitoring pipeline if not already running
        self.pipeline.ensure_started().await?;

        Ok(BuildMonitor {
            system: self.clone(),
            span,
            package_name: package_name.to_string(),
            version: version.clone(),
        })
    }

    /// Get current metrics snapshot
    pub async fn get_metrics_snapshot(&self) -> Result<MetricsSnapshot, Error> {
        self.metrics.get_snapshot().await
    }

    /// Get aggregated statistics
    pub async fn get_statistics(&self) -> Result<StatisticalSummary, Error> {
        let aggregator = self.aggregator.read().await;
        Ok(aggregator.get_summary())
    }

    /// Export metrics to a specific format
    pub async fn export_metrics(&self, format: ExportFormat) -> Result<Vec<u8>, Error> {
        let snapshot = self.get_metrics_snapshot().await?;
        match format {
            ExportFormat::Prometheus => export::prometheus::export(&snapshot),
            ExportFormat::StatsD => export::statsd::export(&snapshot),
            ExportFormat::Json => export::json::export(&snapshot),
        }
    }

    /// Shutdown the monitoring system
    pub async fn shutdown(&self) -> Result<(), Error> {
        self.pipeline.shutdown().await?;
        self.telemetry.stop_collection().await?;
        self.tracing.flush().await?;
        Ok(())
    }
}

/// Monitor for a specific build operation
pub struct BuildMonitor {
    system: MonitoringSystem,
    span: BuildSpan,
    package_name: String,
    version: sps2_types::Version,
}

impl BuildMonitor {
    /// Create a disabled monitor (no-op)
    fn disabled() -> Self {
        // Create a dummy monitoring system for disabled state
        let (tx, _rx) = sps2_events::channel();
        let config = Arc::new(MonitoringConfig {
            enabled: false,
            ..Default::default()
        });

        // Create minimal components for disabled state
        let telemetry = Arc::new(TelemetryCollector::new(config.clone()).unwrap());
        let metrics = Arc::new(MetricsCollector::new(config.clone()).unwrap());
        let aggregator = Arc::new(RwLock::new(MetricsAggregator::new(config.clone())));
        let tracing = Arc::new(TracingCollector::new(config.clone()).unwrap());

        // Use blocking task for initialization in non-async context
        let pipeline = std::thread::spawn({
            let config = config.clone();
            let metrics = metrics.clone();
            let aggregator = aggregator.clone();
            let tx = tx.clone();
            move || {
                tokio::runtime::Handle::current().block_on(async {
                    let pipeline_config = PipelineConfig::from_monitoring_config(&config);
                    Arc::new(
                        MonitoringPipeline::new(pipeline_config, metrics, aggregator, tx)
                            .await
                            .unwrap(),
                    )
                })
            }
        })
        .join()
        .unwrap();

        let system = MonitoringSystem {
            config,
            telemetry,
            metrics,
            aggregator,
            tracing,
            pipeline,
            tx,
        };

        Self {
            system,
            span: BuildSpan::disabled(),
            package_name: String::new(),
            version: sps2_types::Version::new(0, 0, 0),
        }
    }

    /// Record a build step
    pub async fn record_step(&self, step_name: &str) -> Result<(), Error> {
        if !self.system.config.enabled {
            return Ok(());
        }

        self.span.add_event(step_name).await?;
        self.system
            .metrics
            .record_step(&self.span.context, step_name)
            .await?;
        Ok(())
    }

    /// Record resource usage
    pub async fn record_resources(&self) -> Result<(), Error> {
        if !self.system.config.enabled {
            return Ok(());
        }

        let resources = self.system.telemetry.collect_resources().await?;
        self.system
            .metrics
            .record_resources(&self.span.context, &resources)
            .await?;
        Ok(())
    }

    /// Complete the build monitoring
    pub async fn complete(self, success: bool) -> Result<(), Error> {
        if !self.system.config.enabled {
            return Ok(());
        }

        // Record final metrics
        let duration = self.span.duration();
        self.system
            .metrics
            .record_build_completion(&self.span.context, success, duration)
            .await?;

        // Send completion event using existing build events
        if success {
            let _ = self.system.tx.send(Event::BuildCompleted {
                package: self.package_name.clone(),
                version: self.version.clone(),
                path: std::path::PathBuf::new(), // Monitoring doesn't know the path
            });
        } else {
            let _ = self.system.tx.send(Event::BuildFailed {
                package: self.package_name.clone(),
                version: self.version.clone(),
                error: "Build monitoring detected failure".to_string(),
            });
        }

        // Complete the span
        self.span.complete(success).await?;

        Ok(())
    }
}

/// Export format for metrics
#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    /// Prometheus text format
    Prometheus,
    /// StatsD format
    StatsD,
    /// JSON format
    Json,
}

/// Export utilities
mod export {
    pub mod prometheus {
        use crate::monitoring::metrics::MetricsSnapshot;
        use sps2_errors::Error;

        pub fn export(_snapshot: &MetricsSnapshot) -> Result<Vec<u8>, Error> {
            // TODO: Implement Prometheus export
            Ok(Vec::new())
        }
    }

    pub mod statsd {
        use crate::monitoring::metrics::MetricsSnapshot;
        use sps2_errors::Error;

        pub fn export(_snapshot: &MetricsSnapshot) -> Result<Vec<u8>, Error> {
            // TODO: Implement StatsD export
            Ok(Vec::new())
        }
    }

    pub mod json {
        use crate::monitoring::metrics::MetricsSnapshot;
        use sps2_errors::Error;

        pub fn export(_snapshot: &MetricsSnapshot) -> Result<Vec<u8>, Error> {
            // TODO: Implement JSON export
            Ok(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_monitoring_system_creation() {
        let (tx, _rx) = sps2_events::channel();
        let config = MonitoringConfig {
            enabled: true,
            ..Default::default()
        };
        let system = MonitoringSystem::new(config, tx).await.unwrap();
        assert!(system.config.enabled);
    }
}

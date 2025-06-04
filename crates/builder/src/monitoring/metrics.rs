// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Metrics collection and storage

use crate::monitoring::config::MonitoringConfig;
use crate::monitoring::telemetry::{ResourceMetrics, SpanContext};
use dashmap::DashMap;
use sps2_errors::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Type of metric
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MetricType {
    /// Counter that only increases
    Counter,
    /// Gauge that can go up or down
    Gauge,
    /// Histogram for distributions
    Histogram,
    /// Summary with percentiles
    Summary,
}

/// A single metric value
#[derive(Clone, Debug)]
pub struct Metric {
    /// Metric name
    pub name: String,
    /// Metric type
    pub metric_type: MetricType,
    /// Metric value
    pub value: f64,
    /// Labels/tags
    pub labels: Vec<(String, String)>,
    /// Timestamp
    pub timestamp: Instant,
}

/// Metrics snapshot for export
#[derive(Clone, Debug)]
pub struct MetricsSnapshot {
    /// All metrics
    pub metrics: Vec<Metric>,
    /// Build-specific metrics
    pub builds: Vec<BuildMetrics>,
    /// Resource usage over time
    pub resources: Vec<ResourceMetrics>,
    /// Snapshot timestamp
    pub timestamp: Instant,
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            metrics: Vec::new(),
            builds: Vec::new(),
            resources: Vec::new(),
            timestamp: Instant::now(),
        }
    }
}

/// Build-specific metrics
#[derive(Clone, Debug)]
pub struct BuildMetrics {
    /// Package name
    pub package: String,
    /// Package version
    pub version: sps2_types::Version,
    /// Build duration
    pub duration: Duration,
    /// Build success
    pub success: bool,
    /// Number of steps
    pub step_count: usize,
    /// Dependencies resolved
    pub dependency_count: usize,
    /// Peak memory usage
    pub peak_memory: u64,
    /// Average CPU usage
    pub avg_cpu: f32,
    /// Total disk I/O
    pub total_disk_io: u64,
    /// Build start time
    pub start_time: Instant,
}

/// Metrics collector
pub struct MetricsCollector {
    config: Arc<MonitoringConfig>,
    /// Current metrics by name
    metrics: Arc<DashMap<String, Metric>>,
    /// Build-specific metrics
    build_metrics: Arc<DashMap<String, BuildMetrics>>,
    /// Resource history
    resource_history: Arc<RwLock<Vec<ResourceMetrics>>>,
    /// Metric history for aggregation
    metric_history: Arc<RwLock<Vec<Metric>>>,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new(config: Arc<MonitoringConfig>) -> Result<Self, Error> {
        Ok(Self {
            config,
            metrics: Arc::new(DashMap::new()),
            build_metrics: Arc::new(DashMap::new()),
            resource_history: Arc::new(RwLock::new(Vec::new())),
            metric_history: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Initialize metrics for a new build
    pub async fn init_build_metrics(&self, context: &SpanContext) -> Result<(), Error> {
        let build_id = context.span_id.clone();

        // Initialize counters
        self.record_counter(&format!("build.{build_id}.steps"), 0.0)
            .await?;
        self.record_counter(&format!("build.{build_id}.dependencies"), 0.0)
            .await?;

        // Initialize gauges
        self.record_gauge(&format!("build.{build_id}.memory"), 0.0)
            .await?;
        self.record_gauge(&format!("build.{build_id}.cpu"), 0.0)
            .await?;

        Ok(())
    }

    /// Record a counter metric
    pub async fn record_counter(&self, name: &str, value: f64) -> Result<(), Error> {
        let metric = Metric {
            name: name.to_string(),
            metric_type: MetricType::Counter,
            value,
            labels: vec![],
            timestamp: Instant::now(),
        };

        self.metrics.insert(name.to_string(), metric.clone());
        self.add_to_history(metric).await;
        Ok(())
    }

    /// Record a gauge metric
    pub async fn record_gauge(&self, name: &str, value: f64) -> Result<(), Error> {
        let metric = Metric {
            name: name.to_string(),
            metric_type: MetricType::Gauge,
            value,
            labels: vec![],
            timestamp: Instant::now(),
        };

        self.metrics.insert(name.to_string(), metric.clone());
        self.add_to_history(metric).await;
        Ok(())
    }

    /// Record a histogram metric
    pub async fn record_histogram(&self, name: &str, value: f64) -> Result<(), Error> {
        let metric = Metric {
            name: name.to_string(),
            metric_type: MetricType::Histogram,
            value,
            labels: vec![],
            timestamp: Instant::now(),
        };

        self.add_to_history(metric).await;
        Ok(())
    }

    /// Record a build step
    pub async fn record_step(&self, context: &SpanContext, _step_name: &str) -> Result<(), Error> {
        let build_id = &context.span_id;

        // Increment step counter
        let counter_name = format!("build.{build_id}.steps");
        if let Some(mut metric) = self.metrics.get_mut(&counter_name) {
            metric.value += 1.0;
        }

        // Record step timing
        self.record_histogram(
            "build.step.duration",
            0.0, // Will be updated when step completes
        )
        .await?;

        Ok(())
    }

    /// Record resource usage
    pub async fn record_resources(
        &self,
        context: &SpanContext,
        resources: &ResourceMetrics,
    ) -> Result<(), Error> {
        let build_id = &context.span_id;

        // Update gauges
        self.record_gauge(
            &format!("build.{build_id}.cpu"),
            f64::from(resources.cpu_usage),
        )
        .await?;
        self.record_gauge(
            &format!("build.{build_id}.memory"),
            resources.memory_usage as f64,
        )
        .await?;

        // Add to history
        let mut history = self.resource_history.write().await;
        history.push(resources.clone());

        // Trim history if needed
        if history.len() > self.config.max_metrics_retention {
            let drain_count = history.len() - self.config.max_metrics_retention;
            history.drain(0..drain_count);
        }

        Ok(())
    }

    /// Record build completion
    pub async fn record_build_completion(
        &self,
        context: &SpanContext,
        success: bool,
        duration: Duration,
    ) -> Result<(), Error> {
        let build_id = &context.span_id;

        // Record completion metrics
        self.record_counter("build.total", 1.0).await?;
        if success {
            self.record_counter("build.success", 1.0).await?;
        } else {
            self.record_counter("build.failure", 1.0).await?;
        }

        self.record_histogram("build.duration", duration.as_secs_f64())
            .await?;

        // Calculate aggregated metrics for this build
        let resources = self.resource_history.read().await;
        let peak_memory = resources.iter().map(|r| r.memory_usage).max().unwrap_or(0);
        let avg_cpu = if resources.is_empty() {
            0.0
        } else {
            resources.iter().map(|r| r.cpu_usage).sum::<f32>() / resources.len() as f32
        };
        let total_disk_io = resources
            .iter()
            .map(|r| r.disk_read_rate + r.disk_write_rate)
            .sum();

        // Store build metrics
        let build_metrics = BuildMetrics {
            package: String::new(),                     // Will be filled by caller
            version: sps2_types::Version::new(0, 0, 0), // Will be filled by caller
            duration,
            success,
            step_count: self
                .metrics
                .get(&format!("build.{build_id}.steps"))
                .map(|m| m.value as usize)
                .unwrap_or(0),
            dependency_count: self
                .metrics
                .get(&format!("build.{build_id}.dependencies"))
                .map(|m| m.value as usize)
                .unwrap_or(0),
            peak_memory,
            avg_cpu,
            total_disk_io,
            start_time: Instant::now() - duration,
        };

        self.build_metrics.insert(build_id.clone(), build_metrics);

        Ok(())
    }

    /// Get a snapshot of all metrics
    pub async fn get_snapshot(&self) -> Result<MetricsSnapshot, Error> {
        let metrics: Vec<_> = self
            .metrics
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        let builds: Vec<_> = self
            .build_metrics
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        let resources = self.resource_history.read().await.clone();

        Ok(MetricsSnapshot {
            metrics,
            builds,
            resources,
            timestamp: Instant::now(),
        })
    }

    /// Add metric to history
    async fn add_to_history(&self, metric: Metric) {
        let mut history = self.metric_history.write().await;
        history.push(metric);

        // Trim history if needed
        if history.len() > self.config.max_metrics_retention {
            let drain_count = history.len() - self.config.max_metrics_retention;
            history.drain(0..drain_count);
        }
    }

    /// Clear all metrics
    pub async fn clear(&self) {
        self.metrics.clear();
        self.build_metrics.clear();
        self.resource_history.write().await.clear();
        self.metric_history.write().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-10;

    fn assert_float_eq(a: f64, b: f64) {
        assert!(
            (a - b).abs() < EPSILON,
            "Expected {a} to be approximately equal to {b}"
        );
    }

    #[tokio::test]
    async fn test_metrics_recording() {
        let config = Arc::new(MonitoringConfig::default());
        let collector = MetricsCollector::new(config).unwrap();

        // Record some metrics
        collector.record_counter("test.counter", 1.0).await.unwrap();
        collector.record_gauge("test.gauge", 42.0).await.unwrap();

        // Get snapshot
        let snapshot = collector.get_snapshot().await.unwrap();
        assert_eq!(snapshot.metrics.len(), 2);

        // Verify metrics
        let counter = snapshot
            .metrics
            .iter()
            .find(|m| m.name == "test.counter")
            .unwrap();
        assert_float_eq(counter.value, 1.0);
        assert_eq!(counter.metric_type, MetricType::Counter);
    }
}

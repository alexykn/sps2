// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Metrics aggregation with statistical analysis

use crate::monitoring::config::MonitoringConfig;
use crate::monitoring::metrics::{Metric, MetricType};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Statistical summary of metrics
#[derive(Clone, Debug)]
pub struct StatisticalSummary {
    /// Per-metric statistics
    pub metrics: HashMap<String, MetricStats>,
    /// Overall build statistics
    pub builds: BuildStats,
    /// Resource usage statistics
    pub resources: ResourceStats,
    /// Summary generation time
    pub timestamp: Instant,
}

impl Default for StatisticalSummary {
    fn default() -> Self {
        Self {
            metrics: HashMap::new(),
            builds: BuildStats::default(),
            resources: ResourceStats::default(),
            timestamp: Instant::now(),
        }
    }
}

/// Statistics for a single metric
#[derive(Clone, Debug, Default)]
pub struct MetricStats {
    /// Number of samples
    pub count: usize,
    /// Minimum value
    pub min: f64,
    /// Maximum value
    pub max: f64,
    /// Average value
    pub mean: f64,
    /// Standard deviation
    pub stddev: f64,
    /// 50th percentile
    pub p50: f64,
    /// 95th percentile
    pub p95: f64,
    /// 99th percentile
    pub p99: f64,
    /// Sum of all values
    pub sum: f64,
}

/// Build statistics
#[derive(Clone, Debug, Default)]
pub struct BuildStats {
    /// Total builds
    pub total: usize,
    /// Successful builds
    pub success: usize,
    /// Failed builds
    pub failure: usize,
    /// Success rate
    pub success_rate: f64,
    /// Average duration
    pub avg_duration: Duration,
    /// Median duration
    pub median_duration: Duration,
    /// 95th percentile duration
    pub p95_duration: Duration,
}

/// Resource usage statistics
#[derive(Clone, Debug, Default)]
pub struct ResourceStats {
    /// Peak CPU usage
    pub peak_cpu: f32,
    /// Average CPU usage
    pub avg_cpu: f32,
    /// Peak memory usage
    pub peak_memory: u64,
    /// Average memory usage
    pub avg_memory: u64,
    /// Total disk I/O
    pub total_disk_io: u64,
    /// Total network I/O
    pub total_network_io: u64,
}

/// Metrics aggregator
pub struct MetricsAggregator {
    config: Arc<MonitoringConfig>,
    /// Time-series data for each metric
    time_series: HashMap<String, TimeSeries>,
    /// Last aggregation time
    last_aggregation: Instant,
}

impl MetricsAggregator {
    /// Create a new aggregator
    pub fn new(config: Arc<MonitoringConfig>) -> Self {
        Self {
            config,
            time_series: HashMap::new(),
            last_aggregation: Instant::now(),
        }
    }

    /// Add metrics to aggregation
    pub fn add_metrics(&mut self, metrics: &[Metric]) {
        for metric in metrics {
            let series = self
                .time_series
                .entry(metric.name.clone())
                .or_insert_with(|| TimeSeries::new(metric.metric_type.clone()));

            series.add_point(metric.timestamp, metric.value);
        }
    }

    /// Perform aggregation if interval has passed
    pub fn maybe_aggregate(&mut self) -> Option<StatisticalSummary> {
        if self.last_aggregation.elapsed() >= self.config.aggregation_interval {
            self.last_aggregation = Instant::now();
            Some(self.aggregate())
        } else {
            None
        }
    }

    /// Force aggregation
    pub fn aggregate(&self) -> StatisticalSummary {
        let mut summary = StatisticalSummary {
            timestamp: Instant::now(),
            ..Default::default()
        };

        // Aggregate each metric
        for (name, series) in &self.time_series {
            if let Some(stats) = series.calculate_stats() {
                summary.metrics.insert(name.clone(), stats);
            }
        }

        // Calculate build statistics
        summary.builds = self.calculate_build_stats();

        // Calculate resource statistics
        summary.resources = self.calculate_resource_stats();

        summary
    }

    /// Get current summary
    pub fn get_summary(&self) -> StatisticalSummary {
        self.aggregate()
    }

    /// Calculate build statistics
    fn calculate_build_stats(&self) -> BuildStats {
        let mut stats = BuildStats::default();

        // Extract build metrics
        let total_builds = self
            .time_series
            .get("build.total")
            .and_then(TimeSeries::latest_value)
            .unwrap_or(0.0) as usize;

        let success_builds = self
            .time_series
            .get("build.success")
            .and_then(TimeSeries::latest_value)
            .unwrap_or(0.0) as usize;

        let failure_builds = self
            .time_series
            .get("build.failure")
            .and_then(TimeSeries::latest_value)
            .unwrap_or(0.0) as usize;

        stats.total = total_builds;
        stats.success = success_builds;
        stats.failure = failure_builds;
        stats.success_rate = if total_builds > 0 {
            success_builds as f64 / total_builds as f64
        } else {
            0.0
        };

        // Duration statistics
        if let Some(duration_series) = self.time_series.get("build.duration") {
            if let Some(duration_stats) = duration_series.calculate_stats() {
                stats.avg_duration = Duration::from_secs_f64(duration_stats.mean);
                stats.median_duration = Duration::from_secs_f64(duration_stats.p50);
                stats.p95_duration = Duration::from_secs_f64(duration_stats.p95);
            }
        }

        stats
    }

    /// Calculate resource statistics
    fn calculate_resource_stats(&self) -> ResourceStats {
        let mut stats = ResourceStats::default();

        // CPU statistics
        if let Some(cpu_series) = self.time_series.get("system.cpu") {
            if let Some(cpu_stats) = cpu_series.calculate_stats() {
                stats.peak_cpu = cpu_stats.max as f32;
                stats.avg_cpu = cpu_stats.mean as f32;
            }
        }

        // Memory statistics
        if let Some(mem_series) = self.time_series.get("system.memory") {
            if let Some(mem_stats) = mem_series.calculate_stats() {
                stats.peak_memory = mem_stats.max as u64;
                stats.avg_memory = mem_stats.mean as u64;
            }
        }

        // I/O statistics
        if let Some(disk_series) = self.time_series.get("system.disk_io") {
            if let Some(disk_stats) = disk_series.calculate_stats() {
                stats.total_disk_io = disk_stats.sum as u64;
            }
        }

        if let Some(net_series) = self.time_series.get("system.network_io") {
            if let Some(net_stats) = net_series.calculate_stats() {
                stats.total_network_io = net_stats.sum as u64;
            }
        }

        stats
    }

    /// Clear old data
    pub fn cleanup(&mut self, max_age: Duration) {
        let cutoff = Instant::now() - max_age;

        for series in self.time_series.values_mut() {
            series.remove_before(cutoff);
        }

        // Remove empty series
        self.time_series.retain(|_, series| !series.is_empty());
    }
}

/// Time series data for a metric
struct TimeSeries {
    metric_type: MetricType,
    points: Vec<(Instant, f64)>,
}

impl TimeSeries {
    fn new(metric_type: MetricType) -> Self {
        Self {
            metric_type,
            points: Vec::new(),
        }
    }

    fn add_point(&mut self, timestamp: Instant, value: f64) {
        self.points.push((timestamp, value));

        // Keep sorted by timestamp
        self.points.sort_by_key(|(t, _)| *t);
    }

    fn latest_value(&self) -> Option<f64> {
        self.points.last().map(|(_, v)| *v)
    }

    fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    fn remove_before(&mut self, cutoff: Instant) {
        self.points.retain(|(t, _)| *t >= cutoff);
    }

    fn calculate_stats(&self) -> Option<MetricStats> {
        if self.points.is_empty() {
            return None;
        }

        let values: Vec<f64> = self.points.iter().map(|(_, v)| *v).collect();
        let count = values.len();
        let sum: f64 = values.iter().sum();
        let mean = sum / count as f64;

        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        // Calculate standard deviation
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64;
        let stddev = variance.sqrt();

        // Calculate percentiles
        let mut sorted = values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let p50 = percentile(&sorted, 0.50);
        let p95 = percentile(&sorted, 0.95);
        let p99 = percentile(&sorted, 0.99);

        Some(MetricStats {
            count,
            min,
            max,
            mean,
            stddev,
            p50,
            p95,
            p99,
            sum,
        })
    }
}

/// Calculate percentile value
fn percentile(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }

    let len = sorted_values.len();

    // Use simple nearest-rank method: index = floor((n-1) * p)
    let index = ((len - 1) as f64 * p).floor() as usize;
    let index = index.min(len - 1);

    sorted_values[index]
}

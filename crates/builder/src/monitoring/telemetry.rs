// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Telemetry collection for build resource usage

use crate::monitoring::config::MonitoringConfig;
use sps2_errors::Error;
use std::sync::Arc;
use std::time::Instant;
use sysinfo::System;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Resource metrics collected by telemetry
#[derive(Clone, Debug)]
pub struct ResourceMetrics {
    /// CPU usage percentage (0-100)
    pub cpu_usage: f32,
    /// Memory usage in bytes
    pub memory_usage: u64,
    /// Total memory available in bytes
    pub memory_total: u64,
    /// Disk read bytes/sec
    pub disk_read_rate: u64,
    /// Disk write bytes/sec
    pub disk_write_rate: u64,
    /// Network received bytes/sec
    pub network_rx_rate: u64,
    /// Network transmitted bytes/sec
    pub network_tx_rate: u64,
    /// Number of active threads
    pub thread_count: usize,
    /// Collection timestamp
    pub timestamp: Instant,
}

impl Default for ResourceMetrics {
    fn default() -> Self {
        Self {
            cpu_usage: 0.0,
            memory_usage: 0,
            memory_total: 0,
            disk_read_rate: 0,
            disk_write_rate: 0,
            network_rx_rate: 0,
            network_tx_rate: 0,
            thread_count: 0,
            timestamp: Instant::now(),
        }
    }
}

/// Telemetry collector for system resources
pub struct TelemetryCollector {
    config: Arc<MonitoringConfig>,
    system: Arc<RwLock<System>>,
    collection_task: RwLock<Option<JoinHandle<()>>>,
    latest_metrics: Arc<RwLock<ResourceMetrics>>,
    context_pid: RwLock<Option<sysinfo::Pid>>,
}

impl TelemetryCollector {
    /// Create a new telemetry collector
    pub fn new(config: Arc<MonitoringConfig>) -> Result<Self, Error> {
        let mut system = System::new_all();
        system.refresh_all();

        Ok(Self {
            config,
            system: Arc::new(RwLock::new(system)),
            collection_task: RwLock::new(None),
            latest_metrics: Arc::new(RwLock::new(ResourceMetrics::default())),
            context_pid: RwLock::new(None),
        })
    }

    /// Start collection for a specific context
    pub async fn start_collection(&self, context: &SpanContext) -> Result<(), Error> {
        if self.config.resources.flags.is_empty() {
            return Ok(()); // Nothing to collect
        }

        // Store the process ID if available
        if let Some(pid) = context.process_id {
            *self.context_pid.write().await = Some(sysinfo::Pid::from(pid as usize));
        }

        // Stop any existing collection
        self.stop_collection().await?;

        // Start new collection task
        let system = self.system.clone();
        let config = self.config.clone();
        let metrics = self.latest_metrics.clone();
        let context_pid = *self.context_pid.read().await;

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.telemetry_interval);

            loop {
                interval.tick().await;

                let mut sys = system.write().await;

                // Refresh what we need
                if config.resources.flags.cpu() {
                    sys.refresh_cpu_all();
                }
                if config.resources.flags.memory() {
                    sys.refresh_memory();
                }
                // Note: Disk and network I/O stats are not available in sysinfo 0.32
                // These features would need custom implementation or a different library

                // Collect metrics
                let mut new_metrics = ResourceMetrics {
                    timestamp: Instant::now(),
                    ..Default::default()
                };

                // CPU usage
                if config.resources.flags.cpu() {
                    new_metrics.cpu_usage = if let Some(pid) = context_pid {
                        // Process-specific CPU
                        sys.process(pid)
                            .map(sysinfo::Process::cpu_usage)
                            .unwrap_or(0.0)
                    } else {
                        // System-wide CPU
                        sys.cpus().iter().map(sysinfo::Cpu::cpu_usage).sum::<f32>()
                            / sys.cpus().len() as f32
                    };
                }

                // Memory usage
                if config.resources.flags.memory() {
                    if let Some(pid) = context_pid {
                        // Process-specific memory
                        if let Some(process) = sys.process(pid) {
                            new_metrics.memory_usage = process.memory();
                            // Thread count not directly available in newer sysinfo
                            new_metrics.thread_count = 1; // Default to 1 for the process
                        }
                    } else {
                        // System-wide memory
                        new_metrics.memory_usage = sys.used_memory();
                    }
                    new_metrics.memory_total = sys.total_memory();
                }

                // Disk and network I/O not available in sysinfo 0.32
                // Set to zero for now
                new_metrics.disk_read_rate = 0;
                new_metrics.disk_write_rate = 0;
                new_metrics.network_rx_rate = 0;
                new_metrics.network_tx_rate = 0;

                // Update latest metrics
                *metrics.write().await = new_metrics;
            }
        });

        *self.collection_task.write().await = Some(handle);
        Ok(())
    }

    /// Stop telemetry collection
    pub async fn stop_collection(&self) -> Result<(), Error> {
        if let Some(handle) = self.collection_task.write().await.take() {
            handle.abort();
        }
        Ok(())
    }

    /// Collect current resource metrics
    pub async fn collect_resources(&self) -> Result<ResourceMetrics, Error> {
        Ok(self.latest_metrics.read().await.clone())
    }

    /// Get a one-time snapshot of resources
    pub async fn snapshot(&self) -> Result<ResourceMetrics, Error> {
        let mut sys = self.system.write().await;
        sys.refresh_all();

        let mut metrics = ResourceMetrics {
            timestamp: Instant::now(),
            ..Default::default()
        };

        // CPU usage
        metrics.cpu_usage =
            sys.cpus().iter().map(sysinfo::Cpu::cpu_usage).sum::<f32>() / sys.cpus().len() as f32;

        // Memory usage
        metrics.memory_usage = sys.used_memory();
        metrics.memory_total = sys.total_memory();

        // Thread count (approximate from process count)
        metrics.thread_count = sys.processes().len();

        Ok(metrics)
    }
}

/// Span context for tracing
#[derive(Clone, Debug)]
pub struct SpanContext {
    /// Trace ID
    pub trace_id: String,
    /// Span ID
    pub span_id: String,
    /// Parent span ID (if any)
    pub parent_span_id: Option<String>,
    /// Process ID (if tracking specific process)
    pub process_id: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_telemetry_snapshot() {
        let config = Arc::new(MonitoringConfig::default());
        let collector = TelemetryCollector::new(config).unwrap();

        let snapshot = collector.snapshot().await.unwrap();
        assert!(snapshot.memory_total > 0);
    }
}

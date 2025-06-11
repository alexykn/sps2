// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Real-time monitoring pipeline for collection and export

use crate::monitoring::aggregator::MetricsAggregator;
use crate::monitoring::metrics::MetricsCollector;
use sps2_errors::Error;
use sps2_events::{Event, EventSender};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::interval;

/// Pipeline configuration
#[derive(Clone, Debug)]
pub struct PipelineConfig {
    /// Collection interval
    pub collection_interval: Duration,
    /// Aggregation interval
    pub aggregation_interval: Duration,
    /// Export interval
    pub export_interval: Duration,
    /// Enable real-time export
    pub realtime_export: bool,
    /// Enable SQLite storage
    pub sqlite_storage: bool,
    /// SQLite database path
    pub database_path: Option<std::path::PathBuf>,
}

impl PipelineConfig {
    /// Create from monitoring config
    pub fn from_monitoring_config(config: &crate::monitoring::config::MonitoringConfig) -> Self {
        Self {
            collection_interval: config.telemetry_interval,
            aggregation_interval: config.aggregation_interval,
            export_interval: Duration::from_secs(300), // 5 minutes
            realtime_export: config.export.prometheus || config.export.statsd,
            sqlite_storage: config.database_path.is_some(),
            database_path: config.database_path.clone(),
        }
    }
}

/// Monitoring pipeline for real-time collection and export
pub struct MonitoringPipeline {
    config: PipelineConfig,
    metrics: Arc<MetricsCollector>,
    aggregator: Arc<RwLock<MetricsAggregator>>,
    tx: EventSender,
    /// Pipeline task handle
    pipeline_task: RwLock<Option<JoinHandle<()>>>,
    /// Export task handle
    export_task: RwLock<Option<JoinHandle<()>>>,
    /// Storage task handle
    storage_task: RwLock<Option<JoinHandle<()>>>,
}

impl MonitoringPipeline {
    /// Create a new monitoring pipeline
    pub async fn new(
        config: PipelineConfig,
        metrics: Arc<MetricsCollector>,
        aggregator: Arc<RwLock<MetricsAggregator>>,
        tx: EventSender,
    ) -> Result<Self, Error> {
        Ok(Self {
            config,
            metrics,
            aggregator,
            tx,
            pipeline_task: RwLock::new(None),
            export_task: RwLock::new(None),
            storage_task: RwLock::new(None),
        })
    }

    /// Ensure the pipeline is started
    pub async fn ensure_started(&self) -> Result<(), Error> {
        // Start collection pipeline
        if self.pipeline_task.read().await.is_none() {
            self.start_collection_pipeline().await?;
        }

        // Start export pipeline if enabled
        if self.config.realtime_export && self.export_task.read().await.is_none() {
            self.start_export_pipeline().await?;
        }

        // Start storage pipeline if enabled
        if self.config.sqlite_storage && self.storage_task.read().await.is_none() {
            self.start_storage_pipeline().await?;
        }

        Ok(())
    }

    /// Start the collection pipeline
    async fn start_collection_pipeline(&self) -> Result<(), Error> {
        let metrics = self.metrics.clone();
        let aggregator = self.aggregator.clone();
        let tx = self.tx.clone();
        let config = self.config.clone();

        let handle = tokio::spawn(async move {
            let mut collection_interval = interval(config.collection_interval);
            let mut aggregation_interval = interval(config.aggregation_interval);

            loop {
                tokio::select! {
                    _ = collection_interval.tick() => {
                        // Collect current metrics snapshot
                        if let Ok(snapshot) = metrics.get_snapshot().await {
                            // Update aggregator
                            let mut agg = aggregator.write().await;
                            agg.add_metrics(&snapshot.metrics);
                        }
                    }

                    _ = aggregation_interval.tick() => {
                        // Perform aggregation
                        let mut agg = aggregator.write().await;
                        if let Some(summary) = agg.maybe_aggregate() {
                            // Send aggregation event as debug log
                            let _ = tx.send(Event::DebugLog {
                                message: format!("Monitoring aggregated {} metrics", summary.metrics.len()),
                                context: std::collections::HashMap::new(),
                            });
                        }
                    }
                }
            }
        });

        *self.pipeline_task.write().await = Some(handle);
        Ok(())
    }

    /// Start the export pipeline
    async fn start_export_pipeline(&self) -> Result<(), Error> {
        let metrics = self.metrics.clone();
        let aggregator = self.aggregator.clone();
        let tx = self.tx.clone();
        let config = self.config.clone();

        let handle = tokio::spawn(async move {
            let mut export_interval = interval(config.export_interval);

            loop {
                export_interval.tick().await;

                // Get current snapshot and summary
                let snapshot = match metrics.get_snapshot().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let _summary = aggregator.read().await.get_summary();

                // Export to configured backends
                // TODO: Implement actual export logic

                // Send export event as debug log
                let _ = tx.send(Event::DebugLog {
                    message: format!(
                        "Monitoring exported {} metrics to prometheus",
                        snapshot.metrics.len()
                    ),
                    context: std::collections::HashMap::new(),
                });
            }
        });

        *self.export_task.write().await = Some(handle);
        Ok(())
    }

    /// Start the storage pipeline
    async fn start_storage_pipeline(&self) -> Result<(), Error> {
        let metrics = self.metrics.clone();
        let aggregator = self.aggregator.clone();
        let tx = self.tx.clone();
        let _config = self.config.clone();

        let handle = tokio::spawn(async move {
            let mut storage_interval = interval(Duration::from_secs(60)); // Store every minute

            // TODO: Initialize SQLite connection

            loop {
                storage_interval.tick().await;

                // Get current data
                let snapshot = match metrics.get_snapshot().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let _summary = aggregator.read().await.get_summary();

                // TODO: Store to SQLite

                // Send storage event as debug log
                let _ = tx.send(Event::DebugLog {
                    message: format!(
                        "Monitoring stored {} records",
                        snapshot.metrics.len() + snapshot.builds.len()
                    ),
                    context: std::collections::HashMap::new(),
                });
            }
        });

        *self.storage_task.write().await = Some(handle);
        Ok(())
    }

    /// Shutdown the pipeline
    pub async fn shutdown(&self) -> Result<(), Error> {
        // Stop all tasks
        if let Some(handle) = self.pipeline_task.write().await.take() {
            handle.abort();
        }

        if let Some(handle) = self.export_task.write().await.take() {
            handle.abort();
        }

        if let Some(handle) = self.storage_task.write().await.take() {
            handle.abort();
        }

        Ok(())
    }
}

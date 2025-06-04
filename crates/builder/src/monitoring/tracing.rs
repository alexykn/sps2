// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Distributed tracing with OpenTelemetry support

use crate::monitoring::config::MonitoringConfig;
use crate::monitoring::telemetry::SpanContext;
use sps2_errors::Error;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

/// Build span for tracing
#[derive(Clone)]
pub struct BuildSpan {
    /// Span context
    pub context: SpanContext,
    /// Start time
    pub start_time: Instant,
    /// End time (if completed)
    pub end_time: Option<Instant>,
    /// Span events
    pub events: Vec<SpanEvent>,
    /// Span attributes
    pub attributes: HashMap<String, String>,
    /// Span status
    pub status: SpanStatus,
    /// Child spans
    pub children: Vec<Arc<BuildSpan>>,
}

impl BuildSpan {
    /// Create a disabled span (no-op)
    pub fn disabled() -> Self {
        Self {
            context: SpanContext {
                trace_id: String::new(),
                span_id: String::new(),
                parent_span_id: None,
                process_id: None,
            },
            start_time: Instant::now(),
            end_time: None,
            events: Vec::new(),
            attributes: HashMap::new(),
            status: SpanStatus::Ok,
            children: Vec::new(),
        }
    }

    /// Add an event to the span
    pub async fn add_event(&self, _name: &str) -> Result<(), Error> {
        // Note: In a real implementation, this would update the span
        // For now, this is a no-op since we can't mutate self
        Ok(())
    }

    /// Add an attribute
    pub fn add_attribute(&mut self, key: &str, value: &str) {
        self.attributes.insert(key.to_string(), value.to_string());
    }

    /// Get span duration
    pub fn duration(&self) -> Duration {
        if let Some(end) = self.end_time {
            end - self.start_time
        } else {
            self.start_time.elapsed()
        }
    }

    /// Complete the span
    pub async fn complete(self, _success: bool) -> Result<(), Error> {
        // Note: In a real implementation, this would update the span
        // For now, this is a no-op since we consume self
        Ok(())
    }
}

/// Span event
#[derive(Clone, Debug)]
pub struct SpanEvent {
    /// Event name
    pub name: String,
    /// Event timestamp
    pub timestamp: Instant,
    /// Event attributes
    pub attributes: HashMap<String, String>,
}

/// Span status
#[derive(Clone, Debug)]
pub enum SpanStatus {
    /// Span completed successfully
    Ok,
    /// Span encountered an error
    Error,
    /// Span is still running
    Running,
}

/// Tracing collector
pub struct TracingCollector {
    config: Arc<MonitoringConfig>,
    /// Active spans
    active_spans: Arc<RwLock<HashMap<String, Arc<RwLock<BuildSpan>>>>>,
    /// Completed spans
    completed_spans: Arc<RwLock<Vec<Arc<RwLock<BuildSpan>>>>>,
    /// Trace exporter
    exporter: Option<TraceExporter>,
}

impl TracingCollector {
    /// Create a new tracing collector
    pub fn new(config: Arc<MonitoringConfig>) -> Result<Self, Error> {
        let exporter = if config.tracing.enabled {
            TraceExporter::new(&config)?
        } else {
            None
        };

        Ok(Self {
            config,
            active_spans: Arc::new(RwLock::new(HashMap::new())),
            completed_spans: Arc::new(RwLock::new(Vec::new())),
            exporter,
        })
    }

    /// Create a new build span
    pub async fn create_build_span(
        &self,
        package_name: &str,
        version: &sps2_types::Version,
    ) -> Result<BuildSpan, Error> {
        if !self.config.tracing.enabled {
            return Ok(BuildSpan::disabled());
        }

        let trace_id = Uuid::new_v4().to_string();
        let span_id = Uuid::new_v4().to_string();

        let mut span = BuildSpan {
            context: SpanContext {
                trace_id: trace_id.clone(),
                span_id: span_id.clone(),
                parent_span_id: None,
                process_id: Some(std::process::id()),
            },
            start_time: Instant::now(),
            end_time: None,
            events: Vec::new(),
            attributes: HashMap::new(),
            status: SpanStatus::Running,
            children: Vec::new(),
        };

        // Add attributes
        span.add_attribute("package.name", package_name);
        span.add_attribute("package.version", &version.to_string());
        span.add_attribute("service.name", &self.config.tracing.service_name);

        // Clone the span before storing
        let return_span = span.clone();

        // Store active span
        let span_arc = Arc::new(RwLock::new(span));
        self.active_spans.write().await.insert(span_id, span_arc);

        Ok(return_span)
    }

    /// Create a child span
    pub async fn create_child_span(
        &self,
        parent: &SpanContext,
        name: &str,
    ) -> Result<BuildSpan, Error> {
        if !self.config.tracing.enabled {
            return Ok(BuildSpan::disabled());
        }

        let span_id = Uuid::new_v4().to_string();

        let span = BuildSpan {
            context: SpanContext {
                trace_id: parent.trace_id.clone(),
                span_id: span_id.clone(),
                parent_span_id: Some(parent.span_id.clone()),
                process_id: parent.process_id,
            },
            start_time: Instant::now(),
            end_time: None,
            events: Vec::new(),
            attributes: HashMap::from([("span.name".to_string(), name.to_string())]),
            status: SpanStatus::Running,
            children: Vec::new(),
        };

        // Clone the span before storing
        let return_span = span.clone();

        // Store active span
        let span_arc = Arc::new(RwLock::new(span));
        self.active_spans
            .write()
            .await
            .insert(span_id, span_arc.clone());

        // Link to parent
        if let Some(parent_span) = self.active_spans.read().await.get(&parent.span_id) {
            // Convert Arc<RwLock<BuildSpan>> to Arc<BuildSpan> for children
            let child_span = Arc::new(return_span.clone());
            parent_span.write().await.children.push(child_span);
        }

        Ok(return_span)
    }

    /// Complete a span
    pub async fn complete_span(&self, span_id: &str, success: bool) -> Result<(), Error> {
        if !self.config.tracing.enabled {
            return Ok(());
        }

        // Remove from active spans
        if let Some(span_arc) = self.active_spans.write().await.remove(span_id) {
            {
                let mut span = span_arc.write().await;
                span.end_time = Some(Instant::now());
                span.status = if success {
                    SpanStatus::Ok
                } else {
                    SpanStatus::Error
                };
            }

            // Export if needed
            if let Some(exporter) = &self.exporter {
                exporter.export_span(&span_arc).await?;
            }

            // Move to completed spans
            self.completed_spans.write().await.push(span_arc);
        }

        Ok(())
    }

    /// Flush all pending traces
    pub async fn flush(&self) -> Result<(), Error> {
        if let Some(exporter) = &self.exporter {
            exporter.flush().await?;
        }
        Ok(())
    }

    /// Get active spans
    pub async fn get_active_spans(&self) -> Vec<Arc<RwLock<BuildSpan>>> {
        self.active_spans.read().await.values().cloned().collect()
    }

    /// Clean up old completed spans
    pub async fn cleanup(&self, max_age: Duration) {
        let cutoff = Instant::now() - max_age;

        let mut completed = self.completed_spans.write().await;
        completed.retain(|span_arc| {
            // Try to get a read lock without waiting
            if let Ok(span) = span_arc.try_read() {
                span.start_time > cutoff
            } else {
                true // Keep if we can't read
            }
        });
    }
}

/// Trace exporter (placeholder for OpenTelemetry integration)
struct TraceExporter {
    config: Arc<MonitoringConfig>,
    // In a real implementation, this would contain the OTLP exporter
}

impl TraceExporter {
    fn new(config: &Arc<MonitoringConfig>) -> Result<Option<Self>, Error> {
        if !config.tracing.enabled {
            return Ok(None);
        }

        // TODO: Initialize OpenTelemetry OTLP exporter
        // For now, return a placeholder
        Ok(Some(Self {
            config: config.clone(),
        }))
    }

    async fn export_span(&self, _span: &Arc<RwLock<BuildSpan>>) -> Result<(), Error> {
        // TODO: Export to OpenTelemetry
        // For now, this is a no-op
        Ok(())
    }

    async fn flush(&self) -> Result<(), Error> {
        // TODO: Flush OpenTelemetry exporter
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_span_creation() {
        let config = Arc::new(MonitoringConfig {
            tracing: crate::monitoring::config::TracingConfig {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        });

        let collector = TracingCollector::new(config).unwrap();

        let span = collector
            .create_build_span("test-package", &sps2_types::Version::new(1, 0, 0))
            .await
            .unwrap();

        assert!(!span.context.trace_id.is_empty());
        assert!(!span.context.span_id.is_empty());
        assert_eq!(
            span.attributes.get("package.name"),
            Some(&"test-package".to_string())
        );
    }
}

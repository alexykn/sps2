// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Monitoring configuration types

use std::path::PathBuf;
use std::time::Duration;

bitflags::bitflags! {
    /// Resource monitoring flags
    #[derive(Clone, Copy, Debug, Default)]
    pub struct ResourceMonitoringFlags: u8 {
        /// Monitor CPU usage
        const CPU = 0b0001;
        /// Monitor memory usage
        const MEMORY = 0b0010;
        /// Monitor disk I/O
        const DISK_IO = 0b0100;
        /// Monitor network I/O
        const NETWORK_IO = 0b1000;
    }
}

impl ResourceMonitoringFlags {
    /// Create flags with only essential monitoring
    #[must_use]
    pub fn essential() -> Self {
        Self::CPU | Self::MEMORY | Self::DISK_IO
    }

    /// Check if CPU monitoring is enabled
    #[must_use]
    pub fn cpu(self) -> bool {
        self.contains(Self::CPU)
    }

    /// Check if memory monitoring is enabled
    #[must_use]
    pub fn memory(self) -> bool {
        self.contains(Self::MEMORY)
    }

    /// Check if disk I/O monitoring is enabled
    #[must_use]
    pub fn disk_io(self) -> bool {
        self.contains(Self::DISK_IO)
    }

    /// Check if network I/O monitoring is enabled
    #[must_use]
    pub fn network_io(self) -> bool {
        self.contains(Self::NETWORK_IO)
    }
}

/// Monitoring configuration
#[derive(Clone, Debug)]
pub struct MonitoringConfig {
    /// Whether monitoring is enabled
    pub enabled: bool,
    /// Monitoring level
    pub level: MonitoringLevel,
    /// Telemetry collection interval
    pub telemetry_interval: Duration,
    /// Metrics aggregation interval
    pub aggregation_interval: Duration,
    /// Maximum number of metrics to retain in memory
    pub max_metrics_retention: usize,
    /// SQLite database path for historical data
    pub database_path: Option<PathBuf>,
    /// Export configuration
    pub export: ExportConfig,
    /// Tracing configuration
    pub tracing: TracingConfig,
    /// Resource monitoring configuration
    pub resources: ResourceConfig,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default
            level: MonitoringLevel::Basic,
            telemetry_interval: Duration::from_secs(5),
            aggregation_interval: Duration::from_secs(60),
            max_metrics_retention: 10_000,
            database_path: None,
            export: ExportConfig::default(),
            tracing: TracingConfig::default(),
            resources: ResourceConfig::default(),
        }
    }
}

impl MonitoringConfig {
    /// Create an enabled configuration with default settings
    #[must_use]
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// Create a configuration for development
    #[must_use]
    pub fn development() -> Self {
        Self {
            enabled: true,
            level: MonitoringLevel::Detailed,
            telemetry_interval: Duration::from_secs(2),
            aggregation_interval: Duration::from_secs(30),
            ..Default::default()
        }
    }

    /// Create a configuration for production
    #[must_use]
    pub fn production() -> Self {
        Self {
            enabled: true,
            level: MonitoringLevel::Basic,
            telemetry_interval: Duration::from_secs(10),
            aggregation_interval: Duration::from_secs(300), // 5 minutes
            database_path: Some(PathBuf::from("/opt/pm/monitoring.db")),
            ..Default::default()
        }
    }

    /// Enable monitoring
    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set monitoring level
    #[must_use]
    pub fn with_level(mut self, level: MonitoringLevel) -> Self {
        self.level = level;
        self
    }

    /// Set database path for historical data
    #[must_use]
    pub fn with_database(mut self, path: PathBuf) -> Self {
        self.database_path = Some(path);
        self
    }

    /// Set export configuration
    #[must_use]
    pub fn with_export(mut self, export: ExportConfig) -> Self {
        self.export = export;
        self
    }
}

/// Monitoring level
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum MonitoringLevel {
    /// Basic metrics only
    Basic,
    /// Standard metrics and traces
    Standard,
    /// Detailed metrics, traces, and profiling
    Detailed,
}

/// Export configuration
#[derive(Clone, Debug, Default)]
pub struct ExportConfig {
    /// Enable Prometheus export
    pub prometheus: bool,
    /// Prometheus endpoint (if enabled)
    pub prometheus_endpoint: Option<String>,
    /// Enable StatsD export
    pub statsd: bool,
    /// StatsD host (if enabled)
    pub statsd_host: Option<String>,
    /// Enable JSON file export
    pub json_export: bool,
    /// JSON export directory
    pub json_export_dir: Option<PathBuf>,
}

/// Tracing configuration
#[derive(Clone, Debug)]
pub struct TracingConfig {
    /// Enable distributed tracing
    pub enabled: bool,
    /// OpenTelemetry endpoint
    pub otlp_endpoint: Option<String>,
    /// Service name for traces
    pub service_name: String,
    /// Sampling rate (0.0 to 1.0)
    pub sampling_rate: f64,
    /// Maximum trace duration to retain
    pub max_trace_duration: Duration,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            otlp_endpoint: None,
            service_name: "sps2-builder".to_string(),
            sampling_rate: 0.1, // 10% sampling by default
            max_trace_duration: Duration::from_secs(3600), // 1 hour
        }
    }
}

/// Resource monitoring configuration
#[derive(Clone, Debug)]
pub struct ResourceConfig {
    /// Monitoring flags
    pub flags: ResourceMonitoringFlags,
    /// CPU sampling interval
    pub cpu_interval: Duration,
    /// Memory sampling interval
    pub memory_interval: Duration,
    /// I/O sampling interval
    pub io_interval: Duration,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            flags: ResourceMonitoringFlags::essential(), // CPU, memory, disk_io enabled; network_io disabled for privacy
            cpu_interval: Duration::from_secs(1),
            memory_interval: Duration::from_secs(5),
            io_interval: Duration::from_secs(10),
        }
    }
}

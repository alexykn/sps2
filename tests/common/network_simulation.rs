//! Network condition simulation for comprehensive download testing
//!
//! This module provides utilities to simulate various network conditions
//! including bandwidth limits, latency, packet loss, and connection instability.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Network simulation configuration
#[derive(Debug, Clone)]
pub struct NetworkSimulationConfig {
    /// Bandwidth limit in bytes per second (None = unlimited)
    pub bandwidth_limit: Option<u64>,
    /// Base latency for requests
    pub base_latency: Duration,
    /// Latency jitter (random variation)
    pub latency_jitter: Duration,
    /// Packet loss probability (0.0 to 1.0)
    pub packet_loss_rate: f64,
    /// Connection timeout probability (0.0 to 1.0)
    pub timeout_rate: f64,
    /// Burst allowance for bandwidth limiting
    pub burst_size: u64,
    /// Simulate network congestion
    pub congestion_probability: f64,
}

impl Default for NetworkSimulationConfig {
    fn default() -> Self {
        Self {
            bandwidth_limit: None,
            base_latency: Duration::from_millis(10),
            latency_jitter: Duration::from_millis(5),
            packet_loss_rate: 0.0,
            timeout_rate: 0.0,
            burst_size: 64 * 1024, // 64KB burst
            congestion_probability: 0.0,
        }
    }
}

/// Network condition presets for common scenarios
pub struct NetworkPresets;

impl NetworkPresets {
    /// Ideal fast connection (gigabit ethernet)
    pub fn gigabit_ethernet() -> NetworkSimulationConfig {
        NetworkSimulationConfig {
            bandwidth_limit: Some(125_000_000), // 1 Gbps = 125 MB/s
            base_latency: Duration::from_millis(1),
            latency_jitter: Duration::from_millis(1),
            packet_loss_rate: 0.001, // 0.1%
            timeout_rate: 0.0001,    // 0.01%
            burst_size: 1024 * 1024, // 1MB burst
            congestion_probability: 0.0,
        }
    }

    /// Cable modem / DSL connection
    pub fn broadband() -> NetworkSimulationConfig {
        NetworkSimulationConfig {
            bandwidth_limit: Some(2_500_000), // 20 Mbps = 2.5 MB/s
            base_latency: Duration::from_millis(20),
            latency_jitter: Duration::from_millis(10),
            packet_loss_rate: 0.01,       // 1%
            timeout_rate: 0.001,          // 0.1%
            burst_size: 256 * 1024,       // 256KB burst
            congestion_probability: 0.05, // 5%
        }
    }

    /// Mobile LTE connection
    pub fn mobile_lte() -> NetworkSimulationConfig {
        NetworkSimulationConfig {
            bandwidth_limit: Some(1_250_000), // 10 Mbps = 1.25 MB/s
            base_latency: Duration::from_millis(50),
            latency_jitter: Duration::from_millis(25),
            packet_loss_rate: 0.02,       // 2%
            timeout_rate: 0.005,          // 0.5%
            burst_size: 128 * 1024,       // 128KB burst
            congestion_probability: 0.15, // 15%
        }
    }

    /// Satellite internet connection
    pub fn satellite() -> NetworkSimulationConfig {
        NetworkSimulationConfig {
            bandwidth_limit: Some(625_000),           // 5 Mbps = 625 KB/s
            base_latency: Duration::from_millis(600), // Geostationary satellite
            latency_jitter: Duration::from_millis(100),
            packet_loss_rate: 0.05,      // 5%
            timeout_rate: 0.02,          // 2%
            burst_size: 64 * 1024,       // 64KB burst
            congestion_probability: 0.3, // 30%
        }
    }

    /// Dial-up modem (for extreme testing)
    pub fn dialup() -> NetworkSimulationConfig {
        NetworkSimulationConfig {
            bandwidth_limit: Some(7_000), // 56k = ~7 KB/s
            base_latency: Duration::from_millis(200),
            latency_jitter: Duration::from_millis(100),
            packet_loss_rate: 0.1,       // 10%
            timeout_rate: 0.05,          // 5%
            burst_size: 1024,            // 1KB burst
            congestion_probability: 0.5, // 50%
        }
    }

    /// Unstable connection with frequent interruptions
    pub fn unstable() -> NetworkSimulationConfig {
        NetworkSimulationConfig {
            bandwidth_limit: Some(500_000), // 4 Mbps = 500 KB/s
            base_latency: Duration::from_millis(100),
            latency_jitter: Duration::from_millis(200),
            packet_loss_rate: 0.15,      // 15%
            timeout_rate: 0.1,           // 10%
            burst_size: 32 * 1024,       // 32KB burst
            congestion_probability: 0.8, // 80%
        }
    }
}

/// Network condition simulator that can be applied to downloads
pub struct NetworkSimulator {
    config: NetworkSimulationConfig,
    bytes_transferred: Arc<AtomicU64>,
    last_reset: Arc<std::sync::Mutex<Instant>>,
}

impl NetworkSimulator {
    /// Create a new network simulator with the given configuration
    pub fn new(config: NetworkSimulationConfig) -> Self {
        Self {
            config,
            bytes_transferred: Arc::new(AtomicU64::new(0)),
            last_reset: Arc::new(std::sync::Mutex::new(Instant::now())),
        }
    }

    /// Simulate network delay before starting a request
    pub async fn simulate_request_delay(&self) -> Result<(), NetworkError> {
        // Check for timeout simulation
        if self.config.timeout_rate > 0.0 && rand::random::<f64>() < self.config.timeout_rate {
            return Err(NetworkError::Timeout);
        }

        // Simulate base latency with jitter
        let jitter_millis = rand::random::<u64>() % self.config.latency_jitter.as_millis() as u64;
        let total_delay = self.config.base_latency + Duration::from_millis(jitter_millis);

        sleep(total_delay).await;
        Ok(())
    }

    /// Simulate bandwidth limiting for a chunk of data
    ///
    /// # Errors
    ///
    /// Returns an error if the network simulation determines the connection should fail.
    pub async fn simulate_transfer(&self, chunk_size: u64) -> Result<(), NetworkError> {
        // Check for packet loss
        if self.config.packet_loss_rate > 0.0
            && rand::random::<f64>() < self.config.packet_loss_rate
        {
            return Err(NetworkError::PacketLoss);
        }

        // Check for congestion
        if self.config.congestion_probability > 0.0
            && rand::random::<f64>() < self.config.congestion_probability
        {
            // Add extra delay for congestion
            let congestion_delay = Duration::from_millis(rand::random::<u64>() % 1000);
            sleep(congestion_delay).await;
        }

        // Apply bandwidth limiting
        if let Some(bandwidth_limit) = self.config.bandwidth_limit {
            let current_bytes = self
                .bytes_transferred
                .fetch_add(chunk_size, Ordering::Relaxed)
                + chunk_size;

            // Reset counter every second for burst allowance
            {
                let mut last_reset = self.last_reset.lock().unwrap();
                if last_reset.elapsed() >= Duration::from_secs(1) {
                    self.bytes_transferred.store(0, Ordering::Relaxed);
                    *last_reset = Instant::now();
                    return Ok(());
                }
            }

            // Check if we've exceeded the burst allowance
            if current_bytes > self.config.burst_size {
                // Calculate delay needed to stay within bandwidth limit
                let bytes_over_burst = current_bytes - self.config.burst_size;
                let delay_seconds = bytes_over_burst as f64 / bandwidth_limit as f64;
                let delay = Duration::from_millis((delay_seconds * 1000.0) as u64);

                if delay > Duration::from_millis(1) {
                    sleep(delay).await;
                }
            }
        }

        Ok(())
    }

    /// Reset the bandwidth counter (useful for testing)
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn reset_bandwidth_counter(&self) {
        self.bytes_transferred.store(0, Ordering::Relaxed);
        let mut last_reset = self.last_reset.lock().unwrap();
        *last_reset = Instant::now();
    }

    /// Get current bandwidth usage statistics
    pub fn get_bandwidth_stats(&self) -> BandwidthStats {
        let last_reset = self.last_reset.lock().unwrap();
        let elapsed = last_reset.elapsed();
        let bytes = self.bytes_transferred.load(Ordering::Relaxed);

        let current_bps = if elapsed.as_secs_f64() > 0.0 {
            bytes as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        BandwidthStats {
            bytes_transferred: bytes,
            elapsed,
            current_bps,
            configured_limit: self.config.bandwidth_limit,
        }
    }
}

/// Network simulation errors
#[derive(Debug, Clone)]
#[allow(dead_code)] // Test infrastructure - not all variants used yet
pub enum NetworkError {
    /// Connection timeout
    Timeout,
    /// Packet loss occurred
    PacketLoss,
    /// Connection interrupted
    Interrupted,
    /// Bandwidth exceeded
    BandwidthExceeded,
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::Timeout => write!(f, "Connection timeout"),
            NetworkError::PacketLoss => write!(f, "Packet loss"),
            NetworkError::Interrupted => write!(f, "Connection interrupted"),
            NetworkError::BandwidthExceeded => write!(f, "Bandwidth limit exceeded"),
        }
    }
}

impl std::error::Error for NetworkError {}

/// Bandwidth usage statistics
#[derive(Debug, Clone)]
#[allow(dead_code)] // Test infrastructure - not all fields used yet
pub struct BandwidthStats {
    /// Total bytes transferred in current window
    pub bytes_transferred: u64,
    /// Time elapsed since last reset
    pub elapsed: Duration,
    /// Current transfer rate in bytes per second
    pub current_bps: f64,
    /// Configured bandwidth limit
    pub configured_limit: Option<u64>,
}

/// Connection quality assessment based on network conditions
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionQuality {
    /// Excellent connection (< 10ms latency, < 0.1% loss)
    Excellent,
    /// Good connection (< 50ms latency, < 1% loss)
    Good,
    /// Fair connection (< 200ms latency, < 5% loss)
    Fair,
    /// Poor connection (< 500ms latency, < 15% loss)
    Poor,
    /// Very poor connection (high latency or high loss)
    VeryPoor,
}

impl NetworkSimulationConfig {
    /// Assess the quality of this network configuration
    pub fn assess_quality(&self) -> ConnectionQuality {
        let latency_ms = self.base_latency.as_millis() as u64;
        let loss_rate = self.packet_loss_rate;

        if latency_ms < 10 && loss_rate < 0.001 {
            ConnectionQuality::Excellent
        } else if latency_ms < 50 && loss_rate < 0.01 {
            ConnectionQuality::Good
        } else if latency_ms < 200 && loss_rate < 0.05 {
            ConnectionQuality::Fair
        } else if latency_ms < 500 && loss_rate < 0.15 {
            ConnectionQuality::Poor
        } else {
            ConnectionQuality::VeryPoor
        }
    }

    /// Estimate expected download time for a given file size
    pub fn estimate_download_time(&self, file_size: u64) -> Duration {
        let base_time = if let Some(bandwidth) = self.bandwidth_limit {
            Duration::from_secs_f64(file_size as f64 / bandwidth as f64)
        } else {
            Duration::from_millis(1) // Assume very fast for unlimited bandwidth
        };

        // Add overhead for latency and retransmissions
        let overhead_factor = 1.0 + self.packet_loss_rate * 2.0; // Rough estimate
        base_time.mul_f64(overhead_factor) + self.base_latency
    }
}

/// Test scenario generator for various network conditions
pub struct NetworkScenarioGenerator;

impl NetworkScenarioGenerator {
    /// Generate test scenarios covering various network conditions
    pub fn generate_test_scenarios() -> Vec<(String, NetworkSimulationConfig)> {
        vec![
            (
                "Gigabit Ethernet".to_string(),
                NetworkPresets::gigabit_ethernet(),
            ),
            ("Broadband Cable".to_string(), NetworkPresets::broadband()),
            ("Mobile LTE".to_string(), NetworkPresets::mobile_lte()),
            (
                "Satellite Internet".to_string(),
                NetworkPresets::satellite(),
            ),
            ("Dial-up Modem".to_string(), NetworkPresets::dialup()),
            (
                "Unstable Connection".to_string(),
                NetworkPresets::unstable(),
            ),
        ]
    }

    /// Generate stress test scenarios with extreme conditions
    pub fn generate_stress_scenarios() -> Vec<(String, NetworkSimulationConfig)> {
        vec![
            (
                "High Latency".to_string(),
                NetworkSimulationConfig {
                    base_latency: Duration::from_secs(2),
                    latency_jitter: Duration::from_secs(1),
                    ..Default::default()
                },
            ),
            (
                "High Packet Loss".to_string(),
                NetworkSimulationConfig {
                    packet_loss_rate: 0.25, // 25%
                    ..Default::default()
                },
            ),
            (
                "Frequent Timeouts".to_string(),
                NetworkSimulationConfig {
                    timeout_rate: 0.2, // 20%
                    ..Default::default()
                },
            ),
            (
                "Severely Limited Bandwidth".to_string(),
                NetworkSimulationConfig {
                    bandwidth_limit: Some(1024), // 1 KB/s
                    burst_size: 512,
                    ..Default::default()
                },
            ),
            (
                "Chaos Mode".to_string(),
                NetworkSimulationConfig {
                    bandwidth_limit: Some(10_000), // 10 KB/s
                    base_latency: Duration::from_millis(500),
                    latency_jitter: Duration::from_millis(1000),
                    packet_loss_rate: 0.3,
                    timeout_rate: 0.15,
                    burst_size: 2048,
                    congestion_probability: 0.9,
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_network_simulator_creation() {
        let config = NetworkSimulationConfig::default();
        let simulator = NetworkSimulator::new(config);

        let stats = simulator.get_bandwidth_stats();
        assert_eq!(stats.bytes_transferred, 0);
    }

    #[tokio::test]
    async fn test_request_delay_simulation() {
        let config = NetworkSimulationConfig {
            base_latency: Duration::from_millis(100),
            latency_jitter: Duration::from_millis(10),
            timeout_rate: 0.0, // No timeouts for this test
            ..Default::default()
        };

        let simulator = NetworkSimulator::new(config);
        let start = Instant::now();

        simulator.simulate_request_delay().await.unwrap();

        let elapsed = start.elapsed();
        // Should take at least base latency time
        assert!(elapsed >= Duration::from_millis(90)); // Allow some variance
    }

    #[tokio::test]
    async fn test_bandwidth_limiting() {
        let config = NetworkSimulationConfig {
            bandwidth_limit: Some(1024), // 1 KB/s
            burst_size: 512,
            ..Default::default()
        };

        let simulator = NetworkSimulator::new(config);
        let start = Instant::now();

        // Transfer more than burst size
        simulator.simulate_transfer(1024).await.unwrap();

        let elapsed = start.elapsed();
        // Should take some time due to bandwidth limiting
        assert!(elapsed >= Duration::from_millis(100));
    }

    #[tokio::test]
    async fn test_packet_loss_simulation() {
        let config = NetworkSimulationConfig {
            packet_loss_rate: 1.0, // 100% loss for deterministic testing
            ..Default::default()
        };

        let simulator = NetworkSimulator::new(config);
        let result = simulator.simulate_transfer(1024).await;

        assert!(matches!(result, Err(NetworkError::PacketLoss)));
    }

    #[tokio::test]
    async fn test_timeout_simulation() {
        let config = NetworkSimulationConfig {
            timeout_rate: 1.0, // 100% timeout for deterministic testing
            ..Default::default()
        };

        let simulator = NetworkSimulator::new(config);
        let result = simulator.simulate_request_delay().await;

        assert!(matches!(result, Err(NetworkError::Timeout)));
    }

    #[test]
    fn test_connection_quality_assessment() {
        let excellent = NetworkSimulationConfig {
            base_latency: Duration::from_millis(5),
            packet_loss_rate: 0.0005,
            ..Default::default()
        };
        assert_eq!(excellent.assess_quality(), ConnectionQuality::Excellent);

        let poor = NetworkSimulationConfig {
            base_latency: Duration::from_millis(300),
            packet_loss_rate: 0.1,
            ..Default::default()
        };
        assert_eq!(poor.assess_quality(), ConnectionQuality::Poor);
    }

    #[test]
    fn test_download_time_estimation() {
        let config = NetworkSimulationConfig {
            bandwidth_limit: Some(1_000_000), // 1 MB/s
            base_latency: Duration::from_millis(50),
            packet_loss_rate: 0.01,
            ..Default::default()
        };

        let file_size = 10_000_000; // 10 MB
        let estimated_time = config.estimate_download_time(file_size);

        // Should be roughly 10 seconds + overhead
        assert!(estimated_time >= Duration::from_secs(10));
        assert!(estimated_time <= Duration::from_secs(15));
    }

    #[test]
    fn test_network_presets() {
        let scenarios = NetworkScenarioGenerator::generate_test_scenarios();
        assert_eq!(scenarios.len(), 6);

        let stress_scenarios = NetworkScenarioGenerator::generate_stress_scenarios();
        assert_eq!(stress_scenarios.len(), 5);

        // Verify that all scenarios are valid
        for (name, config) in scenarios {
            assert!(!name.is_empty());
            assert!(config.packet_loss_rate >= 0.0 && config.packet_loss_rate <= 1.0);
            assert!(config.timeout_rate >= 0.0 && config.timeout_rate <= 1.0);
        }
    }

    #[test]
    fn test_bandwidth_stats() {
        let config = NetworkSimulationConfig::default();
        let simulator = NetworkSimulator::new(config);

        // Simulate some transfer
        simulator.bytes_transferred.store(1024, Ordering::Relaxed);

        let stats = simulator.get_bandwidth_stats();
        assert_eq!(stats.bytes_transferred, 1024);
        assert!(stats.elapsed < Duration::from_secs(1));
    }
}

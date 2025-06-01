//! Test helper utilities for download testing infrastructure
//!
//! This module provides common utilities for setting up test environments,
//! collecting events, measuring performance, and validating results.

use rand::RngCore;
use sps2_events::{Event, EventSender};
use sps2_hash::Hash;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Test environment for download testing
pub struct TestEnvironment {
    /// Temporary directory for test files
    pub temp_dir: TempDir,
    /// Event channel sender
    pub event_sender: EventSender,
    /// Event collector for verification
    pub event_collector: Arc<Mutex<Vec<Event>>>,
    /// Event channel receiver (for manual event collection)
    pub event_receiver: mpsc::UnboundedReceiver<Event>,
    /// Performance metrics collector
    pub metrics: Arc<Mutex<PerformanceMetrics>>,
}

/// Test event collector for verifying event sequences
#[allow(dead_code)] // Test infrastructure - not all methods used yet
pub struct TestEventCollector {
    events: Vec<Event>,
}

impl TestEventCollector {
    #[allow(dead_code)] // Test infrastructure
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    #[allow(dead_code)] // Test infrastructure
    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
    }

    #[allow(dead_code)] // Test infrastructure
    pub fn get_events(&self) -> &[Event] {
        &self.events
    }

    #[allow(dead_code)] // Test infrastructure
    pub fn get_events_cloned(&self) -> Vec<Event> {
        self.events.clone()
    }

    #[allow(dead_code)] // Test infrastructure
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

impl TestEnvironment {
    /// Create a new test environment
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let event_collector = Arc::new(Mutex::new(Vec::new()));
        let metrics = Arc::new(Mutex::new(PerformanceMetrics::new()));

        Ok(Self {
            temp_dir,
            event_sender,
            event_collector,
            event_receiver,
            metrics,
        })
    }

    /// Get a path within the test directory
    pub fn test_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.temp_dir.path().join(path)
    }

    /// Start collecting events in the background
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn start_event_collection(&mut self) {
        let collector = self.event_collector.clone();
        let metrics = self.metrics.clone();
        let mut receiver = std::mem::replace(&mut self.event_receiver, mpsc::unbounded_channel().1);

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                // Update metrics based on event type
                TestEnvironment::update_metrics_from_event(&event, &metrics);

                // Store event for later verification
                {
                    let mut events = collector.lock().unwrap();
                    events.push(event);
                }
            }
        });
    }

    /// Collect all events with a timeout
    pub async fn collect_events_with_timeout(&mut self, timeout_duration: Duration) -> Vec<Event> {
        let mut events = Vec::new();
        let deadline = Instant::now() + timeout_duration;

        while Instant::now() < deadline {
            match timeout(Duration::from_millis(100), self.event_receiver.recv()).await {
                Ok(Some(event)) => {
                    Self::update_metrics_from_event(&event, &self.metrics);
                    events.push(event);
                }
                Ok(None) => break,  // Channel closed
                Err(_) => continue, // Timeout, try again
            }
        }

        events
    }

    /// Get collected events
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn get_events(&self) -> Vec<Event> {
        let events = self.event_collector.lock().unwrap();
        events.clone()
    }

    /// Get performance metrics
    pub fn get_metrics(&self) -> PerformanceMetrics {
        let metrics = self.metrics.lock().unwrap();
        metrics.clone()
    }

    /// Clear collected events and metrics
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn clear(&self) {
        {
            let mut events = self.event_collector.lock().unwrap();
            events.clear();
        }
        {
            let mut metrics = self.metrics.lock().unwrap();
            *metrics = PerformanceMetrics::new();
        }
    }

    /// Update metrics based on event
    fn update_metrics_from_event(event: &Event, metrics: &Arc<Mutex<PerformanceMetrics>>) {
        let mut m = metrics.lock().unwrap();

        match event {
            Event::DownloadStarted { size, .. } => {
                m.downloads_started += 1;
                if let Some(size) = size {
                    m.total_bytes_expected += size;
                }
                m.start_times.push(Instant::now());
            }
            Event::DownloadCompleted { size, .. } => {
                m.downloads_completed += 1;
                m.total_bytes_downloaded += size;
                if let Some(start_time) = m.start_times.pop() {
                    m.download_durations.push(start_time.elapsed());
                }
            }
            Event::DownloadProgress {
                bytes_downloaded,
                total_bytes,
                ..
            } => {
                m.progress_updates += 1;
                m.last_progress_bytes = *bytes_downloaded;
                m.last_progress_total = *total_bytes;
            }
            Event::DownloadResuming { .. } => {
                m.resume_attempts += 1;
            }
            Event::PackageDownloadStarted { .. } => {
                m.package_downloads_started += 1;
            }
            Event::PackageDownloaded { .. } => {
                m.package_downloads_completed += 1;
            }
            Event::DebugLog { message, .. } => {
                if message.contains("retry") || message.contains("retrying") {
                    m.retry_attempts += 1;
                }
                if message.contains("error") || message.contains("failed") {
                    m.error_count += 1;
                }
            }
            _ => {}
        }
    }
}

/// Performance metrics for download testing
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    /// Number of downloads started
    pub downloads_started: u64,
    /// Number of downloads completed
    pub downloads_completed: u64,
    /// Number of package downloads started
    pub package_downloads_started: u64,
    /// Number of package downloads completed
    pub package_downloads_completed: u64,
    /// Total bytes expected to download
    pub total_bytes_expected: u64,
    /// Total bytes actually downloaded
    pub total_bytes_downloaded: u64,
    /// Number of progress updates received
    pub progress_updates: u64,
    /// Number of resume attempts
    pub resume_attempts: u64,
    /// Number of retry attempts
    pub retry_attempts: u64,
    /// Number of errors encountered
    pub error_count: u64,
    /// Last progress bytes reported
    pub last_progress_bytes: u64,
    /// Last progress total reported
    pub last_progress_total: u64,
    /// Individual download durations
    pub download_durations: Vec<Duration>,
    /// Start times for tracking duration
    pub start_times: Vec<Instant>,
}

impl PerformanceMetrics {
    /// Create new performance metrics
    pub fn new() -> Self {
        Self {
            downloads_started: 0,
            downloads_completed: 0,
            package_downloads_started: 0,
            package_downloads_completed: 0,
            total_bytes_expected: 0,
            total_bytes_downloaded: 0,
            progress_updates: 0,
            resume_attempts: 0,
            retry_attempts: 0,
            error_count: 0,
            last_progress_bytes: 0,
            last_progress_total: 0,
            download_durations: Vec::new(),
            start_times: Vec::new(),
        }
    }

    /// Calculate average download speed in bytes per second
    pub fn average_download_speed(&self) -> f64 {
        if self.download_durations.is_empty() {
            return 0.0;
        }

        let total_duration: Duration = self.download_durations.iter().sum();
        if total_duration.as_secs_f64() > 0.0 {
            self.total_bytes_downloaded as f64 / total_duration.as_secs_f64()
        } else {
            0.0
        }
    }

    /// Calculate success rate as a percentage
    pub fn success_rate(&self) -> f64 {
        if self.downloads_started > 0 {
            (self.downloads_completed as f64 / self.downloads_started as f64) * 100.0
        } else {
            0.0
        }
    }

    /// Calculate average download duration
    pub fn average_download_duration(&self) -> Duration {
        if self.download_durations.is_empty() {
            Duration::from_secs(0)
        } else {
            let total: Duration = self.download_durations.iter().sum();
            total / self.download_durations.len() as u32
        }
    }

    /// Get progress tracking accuracy (percentage of expected bytes)
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn progress_accuracy(&self) -> f64 {
        if self.last_progress_total > 0 {
            (self.last_progress_bytes as f64 / self.last_progress_total as f64) * 100.0
        } else {
            0.0
        }
    }

    /// Check if all downloads completed successfully
    pub fn all_successful(&self) -> bool {
        self.downloads_started > 0
            && self.downloads_completed == self.downloads_started
            && self.error_count == 0
    }
}

/// Event verification utilities
pub struct EventVerifier;

impl EventVerifier {
    /// Verify that expected download events were received
    pub fn verify_download_sequence(
        events: &[Event],
        expected_downloads: u32,
    ) -> Result<(), String> {
        let mut start_count = 0;
        let mut complete_count = 0;
        let mut progress_count = 0;

        for event in events {
            match event {
                Event::DownloadStarted { .. } => start_count += 1,
                Event::DownloadCompleted { .. } => complete_count += 1,
                Event::DownloadProgress { .. } => progress_count += 1,
                _ => {}
            }
        }

        if start_count != expected_downloads {
            return Err(format!(
                "Expected {} download starts, got {}",
                expected_downloads, start_count
            ));
        }

        if complete_count != expected_downloads {
            return Err(format!(
                "Expected {} download completions, got {}",
                expected_downloads, complete_count
            ));
        }

        if progress_count == 0 {
            return Err("Expected progress events, got none".to_string());
        }

        Ok(())
    }

    /// Verify that package download events were received
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn verify_package_download_sequence(
        events: &[Event],
        expected_packages: u32,
    ) -> Result<(), String> {
        let mut start_count = 0;
        let mut complete_count = 0;

        for event in events {
            match event {
                Event::PackageDownloadStarted { .. } => start_count += 1,
                Event::PackageDownloaded { .. } => complete_count += 1,
                _ => {}
            }
        }

        if start_count != expected_packages {
            return Err(format!(
                "Expected {} package download starts, got {}",
                expected_packages, start_count
            ));
        }

        if complete_count != expected_packages {
            return Err(format!(
                "Expected {} package download completions, got {}",
                expected_packages, complete_count
            ));
        }

        Ok(())
    }

    /// Verify that resume events were received
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn verify_resume_events(events: &[Event]) -> Result<(), String> {
        let has_resume = events
            .iter()
            .any(|e| matches!(e, Event::DownloadResuming { .. }));

        if !has_resume {
            return Err("Expected resume events, got none".to_string());
        }

        Ok(())
    }

    /// Extract debug messages from events
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn extract_debug_messages(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::DebugLog { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    /// Count events of a specific type
    pub fn count_events_of_type<F>(events: &[Event], predicate: F) -> usize
    where
        F: Fn(&Event) -> bool,
    {
        events.iter().filter(|e| predicate(e)).count()
    }
}

/// File validation utilities
pub struct FileValidator;

impl FileValidator {
    /// Validate that a downloaded file matches expected content
    pub async fn validate_file_content(
        file_path: &Path,
        expected_hash: &Hash,
    ) -> Result<(), String> {
        if !file_path.exists() {
            return Err(format!("File does not exist: {}", file_path.display()));
        }

        let actual_hash = Hash::hash_file(file_path)
            .await
            .map_err(|e| format!("Failed to hash file: {}", e))?;

        if actual_hash != *expected_hash {
            return Err(format!(
                "Hash mismatch: expected {}, got {}",
                expected_hash.to_hex(),
                actual_hash.to_hex()
            ));
        }

        Ok(())
    }

    /// Validate file size
    pub async fn validate_file_size(file_path: &Path, expected_size: u64) -> Result<(), String> {
        if !file_path.exists() {
            return Err(format!("File does not exist: {}", file_path.display()));
        }

        let metadata = tokio::fs::metadata(file_path)
            .await
            .map_err(|e| format!("Failed to get file metadata: {}", e))?;

        let actual_size = metadata.len();
        if actual_size != expected_size {
            return Err(format!(
                "Size mismatch: expected {} bytes, got {} bytes",
                expected_size, actual_size
            ));
        }

        Ok(())
    }

    /// Validate that signature file exists and is non-empty
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub async fn validate_signature_file(signature_path: &Path) -> Result<(), String> {
        if !signature_path.exists() {
            return Err(format!(
                "Signature file does not exist: {}",
                signature_path.display()
            ));
        }

        let metadata = tokio::fs::metadata(signature_path)
            .await
            .map_err(|e| format!("Failed to get signature file metadata: {}", e))?;

        if metadata.len() == 0 {
            return Err("Signature file is empty".to_string());
        }

        Ok(())
    }
}

/// Performance benchmark utilities
pub struct PerformanceBenchmark {
    start_time: Instant,
    benchmarks: HashMap<String, Duration>,
    memory_samples: Vec<u64>,
}

impl PerformanceBenchmark {
    /// Create a new performance benchmark
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            benchmarks: HashMap::new(),
            memory_samples: Vec::new(),
        }
    }

    /// Start timing a specific operation
    pub fn start_timing(&mut self, operation: &str) {
        self.benchmarks
            .insert(format!("{}_start", operation), self.start_time.elapsed());
    }

    /// End timing a specific operation
    pub fn end_timing(&mut self, operation: &str) {
        let end_time = self.start_time.elapsed();
        if let Some(start_time) = self.benchmarks.get(&format!("{}_start", operation)) {
            let duration = end_time - *start_time;
            self.benchmarks.insert(operation.to_string(), duration);
        }
    }

    /// Sample current memory usage
    pub fn sample_memory(&mut self) {
        // In a real implementation, you would use system calls to get actual memory usage
        // For testing purposes, we'll simulate this
        let simulated_memory = std::process::id() as u64 * 1024; // Placeholder
        self.memory_samples.push(simulated_memory);
    }

    /// Get benchmark results
    pub fn get_results(&self) -> BenchmarkResults {
        let total_duration = self.start_time.elapsed();
        let operation_durations: HashMap<String, Duration> = self
            .benchmarks
            .iter()
            .filter(|(k, _)| !k.ends_with("_start"))
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        let max_memory = self.memory_samples.iter().max().copied().unwrap_or(0);
        let avg_memory = if self.memory_samples.is_empty() {
            0
        } else {
            self.memory_samples.iter().sum::<u64>() / self.memory_samples.len() as u64
        };

        BenchmarkResults {
            total_duration,
            operation_durations,
            max_memory_usage: max_memory,
            average_memory_usage: avg_memory,
            memory_samples: self.memory_samples.clone(),
        }
    }
}

impl Default for PerformanceBenchmark {
    fn default() -> Self {
        Self::new()
    }
}

/// Benchmark results
#[derive(Debug, Clone)]
#[allow(dead_code)] // Test infrastructure - not all fields used yet
pub struct BenchmarkResults {
    /// Total test duration
    pub total_duration: Duration,
    /// Duration of individual operations
    pub operation_durations: HashMap<String, Duration>,
    /// Maximum memory usage observed
    pub max_memory_usage: u64,
    /// Average memory usage
    pub average_memory_usage: u64,
    /// All memory usage samples
    pub memory_samples: Vec<u64>,
}

impl BenchmarkResults {
    /// Get the duration of a specific operation
    pub fn get_operation_duration(&self, operation: &str) -> Option<Duration> {
        self.operation_durations.get(operation).copied()
    }

    /// Get throughput in bytes per second for an operation
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn get_throughput(&self, operation: &str, bytes_processed: u64) -> Option<f64> {
        self.get_operation_duration(operation).map(|duration| {
            if duration.as_secs_f64() > 0.0 {
                bytes_processed as f64 / duration.as_secs_f64()
            } else {
                0.0
            }
        })
    }

    /// Check if performance meets minimum requirements
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn meets_performance_requirements(
        &self,
        _min_throughput_bps: f64,
        max_memory_mb: u64,
    ) -> bool {
        let actual_max_memory_mb = self.max_memory_usage / (1024 * 1024);

        // For throughput, we'd need to know the bytes processed - this is a simplified check
        actual_max_memory_mb <= max_memory_mb
    }
}

/// Utility for creating test data with specific characteristics
pub struct TestDataGenerator;

impl TestDataGenerator {
    /// Create test data with a specific pattern
    pub fn create_test_data(size: usize, pattern: TestDataPattern) -> Vec<u8> {
        match pattern {
            TestDataPattern::Random => {
                use rand::RngCore;
                let mut data = vec![0u8; size];
                rand::thread_rng().fill_bytes(&mut data);
                data
            }
            TestDataPattern::Sequential => (0..size).map(|i| (i % 256) as u8).collect(),
            TestDataPattern::Zeros => vec![0u8; size],
            TestDataPattern::Ones => vec![0xFF; size],
            TestDataPattern::Alternating => (0..size)
                .map(|i| if i % 2 == 0 { 0xAA } else { 0x55 })
                .collect(),
            TestDataPattern::Mixed => {
                let mut data = Vec::with_capacity(size);
                for i in 0..size {
                    if i % 8 == 0 {
                        data.push(rand::random::<u8>());
                    } else {
                        data.push((i % 256) as u8);
                    }
                }
                data
            }
            TestDataPattern::Text => {
                let text_chars =
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .,;:!?\n";
                (0..size)
                    .map(|i| text_chars[i % text_chars.len()])
                    .collect()
            }
            TestDataPattern::Binary => {
                let mut data = vec![0u8; size];
                rand::thread_rng().fill_bytes(&mut data);
                // Add some null bytes
                for i in (0..size).step_by(16) {
                    if i < size {
                        data[i] = 0;
                    }
                }
                data
            }
        }
    }

    /// Create a file with test data
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub async fn create_test_file(
        path: &Path,
        size: usize,
        pattern: TestDataPattern,
    ) -> Result<Hash, Box<dyn std::error::Error>> {
        let data = Self::create_test_data(size, pattern);
        tokio::fs::write(path, &data).await?;
        Ok(Hash::from_data(&data))
    }
}

/// Patterns for test data generation
#[derive(Debug, Clone)]
#[allow(dead_code)] // Test infrastructure - not all variants used yet
pub enum TestDataPattern {
    /// Random bytes
    Random,
    /// Sequential bytes (0, 1, 2, ..., 255, 0, 1, ...)
    Sequential,
    /// All zeros
    Zeros,
    /// All ones (0xFF)
    Ones,
    /// Alternating pattern (0xAA, 0x55, 0xAA, 0x55, ...)
    Alternating,
    /// Mixed pattern (combination of random and sequential)
    Mixed,
    /// Text-like pattern
    Text,
    /// Binary pattern with occasional null bytes
    Binary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_environment_creation() {
        let env = TestEnvironment::new().unwrap();
        assert!(env.temp_dir.path().exists());

        let test_path = env.test_path("test.txt");
        assert!(test_path.starts_with(env.temp_dir.path()));
    }

    #[tokio::test]
    async fn test_event_collection() {
        let mut env = TestEnvironment::new().unwrap();

        // Send some test events
        env.event_sender
            .send(Event::DownloadStarted {
                url: "test".to_string(),
                size: Some(1024),
            })
            .unwrap();

        env.event_sender
            .send(Event::DownloadCompleted {
                url: "test".to_string(),
                size: 1024,
            })
            .unwrap();

        // Collect events with timeout
        let events = env
            .collect_events_with_timeout(Duration::from_millis(100))
            .await;
        assert_eq!(events.len(), 2);

        let metrics = env.get_metrics();
        assert_eq!(metrics.downloads_started, 1);
        assert_eq!(metrics.downloads_completed, 1);
    }

    #[test]
    fn test_performance_metrics() {
        let mut metrics = PerformanceMetrics::new();
        metrics.downloads_started = 10;
        metrics.downloads_completed = 8;
        metrics.total_bytes_downloaded = 1024;
        metrics.download_durations = vec![Duration::from_secs(1), Duration::from_secs(2)];

        assert_eq!(metrics.success_rate(), 80.0);
        assert_eq!(metrics.average_download_speed(), 1024.0 / 3.0); // Total duration is 3 seconds
        assert_eq!(
            metrics.average_download_duration(),
            Duration::from_millis(1500)
        );
        assert!(!metrics.all_successful());
    }

    #[test]
    fn test_event_verification() {
        let events = vec![
            Event::DownloadStarted {
                url: "test1".to_string(),
                size: Some(1024),
            },
            Event::DownloadProgress {
                url: "test1".to_string(),
                bytes_downloaded: 512,
                total_bytes: 1024,
            },
            Event::DownloadCompleted {
                url: "test1".to_string(),
                size: 1024,
            },
            Event::DownloadStarted {
                url: "test2".to_string(),
                size: Some(2048),
            },
            Event::DownloadProgress {
                url: "test2".to_string(),
                bytes_downloaded: 2048,
                total_bytes: 2048,
            },
            Event::DownloadCompleted {
                url: "test2".to_string(),
                size: 2048,
            },
        ];

        assert!(EventVerifier::verify_download_sequence(&events, 2).is_ok());
        assert!(EventVerifier::verify_download_sequence(&events, 1).is_err());

        let progress_count = EventVerifier::count_events_of_type(&events, |e| {
            matches!(e, Event::DownloadProgress { .. })
        });
        assert_eq!(progress_count, 2);
    }

    #[tokio::test]
    async fn test_file_validation() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.dat");

        let test_data = b"Hello, World!";
        tokio::fs::write(&file_path, test_data).await.unwrap();

        let hash = Hash::from_data(test_data);
        assert!(FileValidator::validate_file_content(&file_path, &hash)
            .await
            .is_ok());
        assert!(
            FileValidator::validate_file_size(&file_path, test_data.len() as u64)
                .await
                .is_ok()
        );

        // Test with wrong hash
        let wrong_hash = Hash::from_data(b"Wrong data");
        assert!(
            FileValidator::validate_file_content(&file_path, &wrong_hash)
                .await
                .is_err()
        );

        // Test with wrong size
        assert!(FileValidator::validate_file_size(&file_path, 999)
            .await
            .is_err());
    }

    #[test]
    fn test_performance_benchmark() {
        let mut benchmark = PerformanceBenchmark::new();

        benchmark.start_timing("operation1");
        std::thread::sleep(Duration::from_millis(10));
        benchmark.end_timing("operation1");

        benchmark.sample_memory();

        let results = benchmark.get_results();
        assert!(results.get_operation_duration("operation1").unwrap() >= Duration::from_millis(10));
        assert!(!results.memory_samples.is_empty());
    }

    #[test]
    fn test_test_data_generation() {
        let random_data = TestDataGenerator::create_test_data(100, TestDataPattern::Random);
        assert_eq!(random_data.len(), 100);

        let sequential_data = TestDataGenerator::create_test_data(10, TestDataPattern::Sequential);
        assert_eq!(sequential_data, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

        let zeros_data = TestDataGenerator::create_test_data(5, TestDataPattern::Zeros);
        assert_eq!(zeros_data, vec![0, 0, 0, 0, 0]);

        let ones_data = TestDataGenerator::create_test_data(5, TestDataPattern::Ones);
        assert_eq!(ones_data, vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    }
}

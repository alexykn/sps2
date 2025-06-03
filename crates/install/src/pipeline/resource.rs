//! Resource management and coordination for the pipeline

use crossbeam::queue::SegQueue;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Resource pools and coordination for pipeline operations
#[derive(Debug)]
pub struct ResourceManager {
    /// Concurrency control semaphores
    pub download_semaphore: Arc<Semaphore>,
    pub decompress_semaphore: Arc<Semaphore>,
    pub validation_semaphore: Arc<Semaphore>,
    /// Memory usage tracking
    pub memory_usage: Arc<AtomicU64>,
    /// Memory limit for operations
    #[allow(dead_code)]
    pub memory_limit: u64,
    /// Resource pools
    pub temp_files: Arc<SegQueue<PathBuf>>,
}

impl ResourceManager {
    /// Create a new resource manager with the given limits
    pub fn new(
        max_downloads: usize,
        max_decompressions: usize,
        max_validations: usize,
        memory_limit: u64,
    ) -> Self {
        Self {
            download_semaphore: Arc::new(Semaphore::new(max_downloads)),
            decompress_semaphore: Arc::new(Semaphore::new(max_decompressions)),
            validation_semaphore: Arc::new(Semaphore::new(max_validations)),
            memory_usage: Arc::new(AtomicU64::new(0)),
            memory_limit,
            temp_files: Arc::new(SegQueue::new()),
        }
    }

    /// Check if memory usage is within limits
    ///
    /// # Returns
    ///
    /// Returns true if current memory usage is below the configured limit
    #[allow(dead_code)]
    pub fn check_memory_limit(&self) -> bool {
        self.memory_usage.load(std::sync::atomic::Ordering::Relaxed) < self.memory_limit
    }

    /// Get current memory usage in bytes
    #[allow(dead_code)]
    pub fn current_memory_usage(&self) -> u64 {
        self.memory_usage.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get memory limit in bytes
    #[allow(dead_code)]
    pub fn memory_limit(&self) -> u64 {
        self.memory_limit
    }

    /// Clean up temporary files
    pub async fn cleanup_temp_files(&self) {
        while let Some(temp_path) = self.temp_files.pop() {
            let _ = tokio::fs::remove_file(temp_path).await;
        }
    }
}

//! Pipeline configuration and resource limits

use std::time::Duration;

/// Configuration for the parallel pipeline
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Maximum concurrent downloads (default: 4)
    pub max_downloads: usize,
    /// Maximum concurrent decompressions (default: 2)
    pub max_decompressions: usize,
    /// Maximum concurrent validations (default: 3)
    pub max_validations: usize,
    /// Buffer size for streaming operations (default: 256KB)
    pub buffer_size: usize,
    /// Memory limit for concurrent operations (default: 100MB)
    pub memory_limit: u64,
    /// Timeout for individual operations (default: 10 minutes)
    pub operation_timeout: Duration,
    /// Enable streaming download-to-decompress optimization
    pub enable_streaming: bool,
    /// Cleanup timeout for failed operations (default: 5 seconds)
    pub cleanup_timeout: Duration,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_downloads: 4,
            max_decompressions: 2,
            max_validations: 3,
            buffer_size: 256 * 1024,                     // 256KB
            memory_limit: 100 * 1024 * 1024,             // 100MB
            operation_timeout: Duration::from_secs(600), // 10 minutes
            enable_streaming: true,
            cleanup_timeout: Duration::from_secs(5),
        }
    }
}

//! Configuration structures for package downloads

use sps2_hash::Hash;
use sps2_types::Version;
use std::path::PathBuf;
use std::time::Duration;

use sps2_config::ResourceManager;
use std::sync::Arc;

/// Configuration for package downloads
#[derive(Clone, Debug)]
pub struct PackageDownloadConfig {
    /// Maximum file size allowed (default: 2GB)
    pub max_file_size: u64,
    /// Buffer size for streaming (default: 128KB)
    pub buffer_size: usize,
    /// Maximum number of concurrent downloads (default: 4)
    pub max_concurrent: usize,
    /// Retry configuration
    pub retry_config: RetryConfig,
    /// Timeout for individual chunks (default: 30s)
    pub chunk_timeout: Duration,
    /// Minimum chunk size for resumable downloads (default: 1MB)
    pub min_chunk_size: u64,
    /// Resource manager
    pub resources: Arc<ResourceManager>,
}

impl Default for PackageDownloadConfig {
    fn default() -> Self {
        Self {
            max_file_size: 2 * 1024 * 1024 * 1024, // 2GB
            buffer_size: 128 * 1024,               // 128KB
            max_concurrent: 4,
            retry_config: RetryConfig::default(),
            chunk_timeout: Duration::from_secs(30),
            min_chunk_size: 1024 * 1024, // 1MB
            resources: Arc::new(ResourceManager::default()),
        }
    }
}

/// Retry configuration for downloads
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries
    pub max_retries: u32,
    /// Initial backoff delay
    pub initial_delay: Duration,
    /// Maximum backoff delay
    pub max_delay: Duration,
    /// Backoff multiplier
    pub backoff_multiplier: f64,
    /// Jitter factor (0.0 to 1.0)
    pub jitter_factor: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter_factor: 0.1,
        }
    }
}

/// Request for downloading a package
#[derive(Debug, Clone)]
pub struct PackageDownloadRequest {
    pub name: String,
    pub version: Version,
    pub package_url: String,
    pub signature_url: Option<String>,
    pub expected_hash: Option<Hash>,
}

/// Result of a package download operation
#[derive(Debug)]
pub struct PackageDownloadResult {
    pub package_path: PathBuf,
    pub signature_path: Option<PathBuf>,
    pub hash: Hash,
    pub size: u64,
    pub download_time: Duration,
    pub signature_verified: bool,
}

/// Parameters for streaming download with unified progress tracking
pub(super) struct StreamParams<'a> {
    pub total_size: u64,
    pub expected_hash: Option<&'a Hash>,
    pub event_sender: &'a sps2_events::EventSender,
    /// URL being downloaded - used for timeout error reporting
    pub url: &'a str,
    pub progress_tracker_id: String,
    /// Optional parent progress ID - reserved for future parent-child coordination features
    #[allow(dead_code)]
    pub parent_progress_id: Option<String>,
    pub progress_manager: Option<&'a sps2_events::ProgressManager>,
}

/// Result of a download operation
#[derive(Debug)]
pub struct DownloadResult {
    pub hash: Hash,
    pub size: u64,
}

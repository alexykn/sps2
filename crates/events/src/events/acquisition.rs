use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;
use std::time::Duration;

/// Package acquisition domain events - higher-level package acquisition from various sources
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AcquisitionEvent {
    /// Package acquisition started
    Started {
        package: String,
        version: Version,
        source: AcquisitionSource,
        destination: PathBuf,
        verification_required: bool,
    },

    /// Package acquisition completed successfully
    Completed {
        package: String,
        version: Version,
        source: AcquisitionSource,
        final_path: PathBuf,
        size: u64,
        duration: Duration,
        verification_passed: bool,
    },

    /// Package acquisition failed
    Failed {
        package: String,
        version: Version,
        source: AcquisitionSource,
        error: String,
        retry_possible: bool,
        partial_download: bool,
    },

    /// Remote download acquisition started
    DownloadStarted {
        package: String,
        version: Version,
        url: String,
        resume_possible: bool,
        expected_size: Option<u64>,
    },

    /// Download progress update
    DownloadProgress {
        package: String,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        current_speed: f64,
        eta: Option<Duration>,
    },

    /// Remote download completed
    DownloadCompleted {
        package: String,
        version: Version,
        url: String,
        final_size: u64,
        average_speed: f64,
        total_time: Duration,
    },

    /// Local file acquisition started
    LocalFileProcessingStarted {
        package: String,
        version: Option<Version>, // May be unknown initially
        file_path: PathBuf,
        file_size: u64,
    },

    /// Local file validation progress
    LocalFileValidationProgress {
        file_path: PathBuf,
        validation_type: String,
        progress_percent: f64,
    },

    /// Local file processed successfully
    LocalFileProcessingCompleted {
        package: String,
        version: Version,
        source_path: PathBuf,
        destination_path: PathBuf,
        validation_results: Vec<ValidationResult>,
    },

    /// Local file processing failed
    LocalFileProcessingFailed {
        file_path: PathBuf,
        error: String,
        validation_failures: Vec<String>,
    },

    /// Cache lookup started
    CacheLookupStarted {
        package: String,
        version: Version,
        cache_locations: Vec<PathBuf>,
    },

    /// Cache hit found
    CacheHit {
        package: String,
        version: Version,
        cache_location: PathBuf,
        cache_age: Duration,
        needs_refresh: bool,
    },

    /// Cache miss, need to acquire from source
    CacheMiss {
        package: String,
        version: Version,
        searched_locations: Vec<PathBuf>,
        fallback_source: AcquisitionSource,
    },

    /// Cache storage started
    CacheStorageStarted {
        package: String,
        version: Version,
        source_path: PathBuf,
        cache_location: PathBuf,
    },

    /// Cache storage completed
    CacheStorageCompleted {
        package: String,
        version: Version,
        cache_location: PathBuf,
        storage_duration: Duration,
    },

    /// Package verification started
    VerificationStarted {
        package: String,
        version: Version,
        file_path: PathBuf,
        verification_types: Vec<VerificationType>,
    },

    /// Verification progress update
    VerificationProgress {
        package: String,
        verifications_completed: usize,
        total_verifications: usize,
        current_verification: VerificationType,
    },

    /// Package verification completed
    VerificationCompleted {
        package: String,
        version: Version,
        file_path: PathBuf,
        verification_results: Vec<ValidationResult>,
        overall_passed: bool,
    },

    /// Package verification failed
    VerificationFailed {
        package: String,
        version: Version,
        file_path: PathBuf,
        failed_verification: VerificationType,
        error: String,
        security_risk: bool,
    },

    /// Checksum verification started
    ChecksumVerificationStarted {
        package: String,
        version: Version,
        algorithm: String,
        expected_hash: String,
    },

    /// Checksum verification completed
    ChecksumVerificationCompleted {
        package: String,
        version: Version,
        algorithm: String,
        expected_hash: String,
        computed_hash: String,
        verification_time: Duration,
        matched: bool,
    },

    /// Checksum mismatch detected
    ChecksumMismatch {
        package: String,
        version: Version,
        algorithm: String,
        expected: String,
        actual: String,
        action_taken: String, // "quarantined", "deleted", "marked_suspicious"
    },

    /// Signature verification started
    SignatureVerificationStarted {
        package: String,
        version: Version,
        signature_file: PathBuf,
        public_key_source: String,
    },

    /// Signature verification completed
    SignatureVerificationCompleted {
        package: String,
        version: Version,
        signature_valid: bool,
        signer_identity: Option<String>,
        verification_time: Duration,
    },

    /// Signature verification failed
    SignatureVerificationFailed {
        package: String,
        version: Version,
        error: String,
        signature_file: Option<PathBuf>,
        security_implications: Vec<String>,
    },

    /// Batch acquisition started
    BatchAcquisitionStarted {
        packages: Vec<(String, Version)>,
        operation_id: String,
        sources: Vec<AcquisitionSource>,
        concurrent_limit: usize,
    },

    /// Batch acquisition progress
    BatchAcquisitionProgress {
        operation_id: String,
        completed_packages: usize,
        failed_packages: usize,
        in_progress_packages: usize,
        remaining_packages: usize,
        total_bytes_acquired: u64,
    },

    /// Batch acquisition completed
    BatchAcquisitionCompleted {
        operation_id: String,
        successful_packages: Vec<(String, Version)>,
        failed_packages: Vec<(String, Version, String)>, // (package, version, error)
        total_duration: Duration,
        total_size: u64,
        cache_hits: usize,
    },

    /// Source availability check
    SourceAvailabilityCheck {
        source: AcquisitionSource,
        available: bool,
        response_time: Option<Duration>,
        error: Option<String>,
    },

    /// Source failover triggered
    SourceFailover {
        package: String,
        version: Version,
        failed_source: AcquisitionSource,
        fallback_source: AcquisitionSource,
        reason: String,
    },

    /// Quota limit approached
    QuotaWarning {
        source: AcquisitionSource,
        quota_type: String, // "bandwidth", "requests", "storage"
        current_usage: u64,
        limit: u64,
        reset_time: Option<Duration>,
    },

    /// Quota limit exceeded
    QuotaExceeded {
        source: AcquisitionSource,
        quota_type: String,
        retry_after: Option<Duration>,
        fallback_available: bool,
    },
}

/// Package acquisition sources
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionSource {
    /// Remote HTTP/HTTPS download
    Remote { url: String, mirror_priority: u8 },
    /// Local file system
    Local { path: PathBuf },
    /// Local package cache
    Cache {
        cache_type: String, // "global", "user", "project"
        location: PathBuf,
    },
    /// Network file share
    NetworkShare {
        protocol: String, // "nfs", "smb", "ftp"
        location: String,
    },
    /// Git repository
    GitRepository {
        url: String,
        branch: Option<String>,
        commit: Option<String>,
    },
}

/// Types of package verification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationType {
    /// BLAKE3/SHA256 checksum verification
    Checksum,
    /// Digital signature verification
    Signature,
    /// Package format validation
    FormatValidation,
    /// Manifest consistency check
    ManifestValidation,
    /// Virus/malware scanning
    SecurityScanning,
    /// File integrity check
    FileIntegrity,
}

/// Verification result for individual checks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub verification_type: VerificationType,
    pub passed: bool,
    pub message: String,
    pub details: Option<String>,
    pub warning_level: ValidationWarningLevel,
}

/// Warning levels for validation results
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationWarningLevel {
    /// No issues
    None,
    /// Minor issues that don't affect functionality
    Low,
    /// Issues that might affect functionality
    Medium,
    /// Serious issues that affect functionality
    High,
    /// Critical security or integrity issues
    Critical,
}

impl AcquisitionEvent {
    /// Create a basic acquisition started event
    pub fn started(
        package: impl Into<String>,
        version: Version,
        source: AcquisitionSource,
    ) -> Self {
        Self::Started {
            package: package.into(),
            version,
            source,
            destination: PathBuf::from("/tmp/sps2-acquisition"),
            verification_required: true,
        }
    }

    /// Create a cache hit event
    pub fn cache_hit(
        package: impl Into<String>,
        version: Version,
        cache_location: PathBuf,
    ) -> Self {
        Self::CacheHit {
            package: package.into(),
            version,
            cache_location,
            cache_age: Duration::from_secs(0),
            needs_refresh: false,
        }
    }

    /// Create a verification completed event
    pub fn verification_completed(
        package: impl Into<String>,
        version: Version,
        file_path: PathBuf,
        passed: bool,
    ) -> Self {
        Self::VerificationCompleted {
            package: package.into(),
            version,
            file_path,
            verification_results: vec![],
            overall_passed: passed,
        }
    }
}

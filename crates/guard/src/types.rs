//! Type definitions for state verification and healing

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;

/// Verification level for state checking
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VerificationLevel {
    /// Quick check - file existence only
    Quick,
    /// Standard check - existence + metadata
    Standard,
    /// Full check - existence + metadata + content hash
    Full,
}

/// Scope for verification operations
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VerificationScope {
    /// Verify all packages and files (current behavior)
    Full,
    /// Verify a specific package by name and version
    Package { name: String, version: String },
    /// Verify multiple specific packages
    Packages { packages: Vec<(String, String)> },
    /// Verify files within a specific directory tree
    Directory { path: PathBuf },
    /// Verify multiple directory trees
    Directories { paths: Vec<PathBuf> },
    /// Verify specific packages and limit orphan detection to specific directories
    Mixed {
        packages: Vec<(String, String)>,
        directories: Vec<PathBuf>,
    },
}

impl Default for VerificationLevel {
    fn default() -> Self {
        Self::Standard
    }
}

impl Default for VerificationScope {
    fn default() -> Self {
        Self::Full
    }
}

/// Coverage information for a scoped verification
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerificationCoverage {
    /// Total number of packages in state
    pub total_packages: usize,
    /// Number of packages actually verified
    pub verified_packages: usize,
    /// Total number of files tracked in database
    pub total_files: usize,
    /// Number of files actually verified
    pub verified_files: usize,
    /// Percentage of packages verified
    pub package_coverage_percent: f64,
    /// Percentage of files verified
    pub file_coverage_percent: f64,
    /// Directories that were checked for orphaned files
    pub orphan_checked_directories: Vec<PathBuf>,
    /// Whether full orphan detection was performed
    pub full_orphan_detection: bool,
}

impl VerificationCoverage {
    /// Create a new verification coverage report
    #[must_use]
    pub fn new(
        total_packages: usize,
        verified_packages: usize,
        total_files: usize,
        verified_files: usize,
        orphan_checked_directories: Vec<PathBuf>,
        full_orphan_detection: bool,
    ) -> Self {
        let package_coverage_percent = if total_packages == 0 {
            100.0
        } else {
            (verified_packages as f64 / total_packages as f64) * 100.0
        };

        let file_coverage_percent = if total_files == 0 {
            100.0
        } else {
            (verified_files as f64 / total_files as f64) * 100.0
        };

        Self {
            total_packages,
            verified_packages,
            total_files,
            verified_files,
            package_coverage_percent,
            file_coverage_percent,
            orphan_checked_directories,
            full_orphan_detection,
        }
    }
}

/// Category of orphaned file
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum OrphanedFileCategory {
    /// Leftover from previous package versions
    Leftover,
    /// User-created file (e.g., config, data)
    UserCreated,
    /// Temporary file that should be cleaned
    Temporary,
    /// System file that should be preserved
    System,
    /// Unknown category - needs investigation
    Unknown,
}

/// Action to take for orphaned files
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanedFileAction {
    /// Remove the file
    Remove,
    /// Preserve the file in place
    Preserve,
    /// Backup the file then remove
    Backup,
}

/// Type of discrepancy found during verification
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Discrepancy {
    /// File expected but not found
    MissingFile {
        package_name: String,
        package_version: String,
        file_path: String,
    },
    /// File exists but has wrong type (file vs directory)
    TypeMismatch {
        package_name: String,
        package_version: String,
        file_path: String,
        expected_directory: bool,
        actual_directory: bool,
    },
    /// File content doesn't match expected hash
    CorruptedFile {
        package_name: String,
        package_version: String,
        file_path: String,
        expected_hash: String,
        actual_hash: String,
    },
    /// File exists but not tracked in database
    OrphanedFile {
        file_path: String,
        category: OrphanedFileCategory,
    },
    /// Python virtual environment missing
    MissingVenv {
        package_name: String,
        package_version: String,
        venv_path: String,
    },
}

/// Result of verification check
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerificationResult {
    /// State ID that was verified
    pub state_id: Uuid,
    /// List of discrepancies found
    pub discrepancies: Vec<Discrepancy>,
    /// Whether verification passed (no discrepancies)
    pub is_valid: bool,
    /// Time taken for verification in milliseconds
    pub duration_ms: u64,
    /// Coverage information for scoped verification
    pub coverage: Option<VerificationCoverage>,
}

impl VerificationResult {
    /// Create a new verification result
    #[must_use]
    pub fn new(state_id: Uuid, discrepancies: Vec<Discrepancy>, duration_ms: u64) -> Self {
        let is_valid = discrepancies.is_empty();
        Self {
            state_id,
            discrepancies,
            is_valid,
            duration_ms,
            coverage: None,
        }
    }

    /// Create a new verification result with coverage information
    #[must_use]
    pub fn with_coverage(
        state_id: Uuid,
        discrepancies: Vec<Discrepancy>,
        duration_ms: u64,
        coverage: VerificationCoverage,
    ) -> Self {
        let is_valid = discrepancies.is_empty();
        Self {
            state_id,
            discrepancies,
            is_valid,
            duration_ms,
            coverage: Some(coverage),
        }
    }
}

/// Cache entry for a verified file
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileCacheEntry {
    /// File path relative to live directory
    pub file_path: String,
    /// Package name this file belongs to
    pub package_name: String,
    /// Package version
    pub package_version: String,
    /// File modification time when last verified
    pub mtime: SystemTime,
    /// File size when last verified
    pub size: u64,
    /// Content hash (only for Full verification level)
    pub content_hash: Option<String>,
    /// Timestamp when this entry was created
    pub verified_at: SystemTime,
    /// Verification level used for this entry
    pub verification_level: VerificationLevel,
    /// Whether file was valid at time of verification
    pub was_valid: bool,
}

/// Cache statistics for monitoring performance
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CacheStats {
    /// Total cache lookups attempted
    pub lookups: u64,
    /// Cache hits (valid entries found)
    pub hits: u64,
    /// Cache misses (no entry or invalidated)
    pub misses: u64,
    /// Number of entries in cache
    pub entry_count: u64,
    /// Total memory usage estimate in bytes
    pub memory_usage_bytes: u64,
    /// Time saved by cache hits in milliseconds
    pub time_saved_ms: u64,
}

impl CacheStats {
    /// Calculate cache hit rate as percentage
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            0.0
        } else {
            (self.hits as f64 / self.lookups as f64) * 100.0
        }
    }
}

/// Context for verification operations to reduce argument count
pub struct VerificationContext<'a> {
    /// State manager for database operations
    pub state_manager: &'a sps2_state::StateManager,
    /// Package store for content verification
    pub store: &'a sps2_store::PackageStore,
    /// Verification cache for performance optimization
    pub cache: &'a mut crate::cache::VerificationCache,
    /// Verification level
    pub level: VerificationLevel,
    /// Current state ID being verified
    pub state_id: &'a uuid::Uuid,
    /// Live directory path
    pub live_path: &'a Path,
}

/// Context for healing operations to reduce argument count
pub struct HealingContext<'a> {
    /// State manager for database operations
    pub state_manager: &'a sps2_state::StateManager,
    /// Package store for content restoration
    pub store: &'a sps2_store::PackageStore,
    /// Event sender for progress reporting
    pub tx: &'a sps2_events::EventSender,
}

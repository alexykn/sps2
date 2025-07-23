//! Type definitions for state verification and healing

use sps2_config::DiscrepancyHandling;
use sps2_errors::{DiscrepancyContext, DiscrepancySeverity, RecommendedAction};
use sps2_events::{EventEmitter, EventSender};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

/// Verification level for state checking
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
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

/// Types of special files that may require custom handling
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum SpecialFileType {
    /// Device file (block device)
    BlockDevice,
    /// Device file (character device)
    CharDevice,
    /// Unix domain socket
    Socket,
    /// Named pipe (FIFO)
    Fifo,
    /// Other special file type
    Other(String),
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
    /// Package content missing from store
    MissingPackageContent {
        package_name: String,
        package_version: String,
    },
    /// Special file type detected that cannot be verified
    UnsupportedSpecialFile {
        package_name: String,
        package_version: String,
        file_path: String,
        file_type: SpecialFileType,
    },
}
/// Helper functions for special file type handling
impl SpecialFileType {
    /// Identify the special file type from file metadata
    pub fn from_metadata(metadata: &std::fs::Metadata) -> Option<Self> {
        use std::os::unix::fs::FileTypeExt;

        let file_type = metadata.file_type();

        if file_type.is_block_device() {
            Some(SpecialFileType::BlockDevice)
        } else if file_type.is_char_device() {
            Some(SpecialFileType::CharDevice)
        } else if file_type.is_socket() {
            Some(SpecialFileType::Socket)
        } else if file_type.is_fifo() {
            Some(SpecialFileType::Fifo)
        } else {
            None
        }
    }

    /// Get a human-readable description of the file type
    pub fn description(&self) -> String {
        match self {
            SpecialFileType::BlockDevice => "block device".to_string(),
            SpecialFileType::CharDevice => "character device".to_string(),
            SpecialFileType::Socket => "Unix domain socket".to_string(),
            SpecialFileType::Fifo => "named pipe (FIFO)".to_string(),
            SpecialFileType::Other(desc) => desc.clone(),
        }
    }

    /// Check if this special file type should be skipped during verification
    pub fn should_skip_verification(&self) -> bool {
        match self {
            // All special file types should be skipped for content verification
            // but their existence should still be tracked
            SpecialFileType::BlockDevice
            | SpecialFileType::CharDevice
            | SpecialFileType::Socket
            | SpecialFileType::Fifo
            | SpecialFileType::Other(_) => true,
        }
    }
}

impl Discrepancy {
    /// Get user-friendly context for this discrepancy
    #[must_use]
    pub fn user_context(&self) -> DiscrepancyContext {
        match self {
            Self::MissingFile { package_name, package_version, file_path } => {
                DiscrepancyContext::new(
                    DiscrepancySeverity::High,
                    RecommendedAction::AutoHeal,
                    format!(
                        "Missing file '{file_path}' from package '{package_name}' v{package_version}. This may cause the application to malfunction."
                    ),
                    format!("Expected file {file_path} from {package_name}:{package_version} not found in filesystem"),
                )
                .with_manual_steps(vec![
                    format!("Reinstall package: sps2 install {}:{}", package_name, package_version),
                    format!("Check if file was manually deleted: {}", file_path),
                    "Run full system verification: sps2 verify --heal".to_string(),
                ])
                .with_prevention_tips(vec![
                    "Avoid manually deleting package files".to_string(),
                    "Use sps2 uninstall to remove packages properly".to_string(),
                    "Run regular verification checks".to_string(),
                ])
                .with_estimated_fix_time(Duration::from_secs(30))
            }
            Self::TypeMismatch { package_name, package_version, file_path, expected_directory, actual_directory } => {
                let expected_type = if *expected_directory { "directory" } else { "file" };
                let actual_type = if *actual_directory { "directory" } else { "file" };
                DiscrepancyContext::new(
                    DiscrepancySeverity::High,
                    RecommendedAction::UserConfirmation,
                    format!(
                        "Type mismatch for '{file_path}' from package '{package_name}' v{package_version}: expected {expected_type} but found {actual_type}. This will prevent proper operation."
                    ),
                    format!("Path {file_path} should be {expected_type} but is {actual_type}"),
                )
                .with_manual_steps(vec![
                    format!("Remove conflicting {}: rm -rf '{}'", actual_type, file_path),
                    format!("Reinstall package: sps2 install {}:{}", package_name, package_version),
                    "Verify system integrity: sps2 verify".to_string(),
                ])
                .with_prevention_tips(vec![
                    "Avoid creating files/directories with package names".to_string(),
                    "Let sps2 manage package directory structure".to_string(),
                ])
                .with_estimated_fix_time(Duration::from_secs(45))
            }
            Self::CorruptedFile { package_name, package_version, file_path, expected_hash, actual_hash } => {
                DiscrepancyContext::new(
                    DiscrepancySeverity::Critical,
                    RecommendedAction::UserConfirmation,
                    format!(
                        "File '{file_path}' from package '{package_name}' v{package_version} has been modified or corrupted. This could indicate tampering, disk errors, or manual modifications."
                    ),
                    format!(
                        "Hash mismatch for {file_path}: expected {expected_hash} but got {actual_hash}"
                    ),
                )
                .with_manual_steps(vec![
                    "Back up the modified file if it contains important changes".to_string(),
                    format!("Restore original file: sps2 install {}:{} --force", package_name, package_version),
                    "Check disk integrity if corruption is suspected".to_string(),
                    "Review recent manual modifications".to_string(),
                ])
                .with_prevention_tips(vec![
                    "Avoid manually editing package files".to_string(),
                    "Use configuration files for customization".to_string(),
                    "Run regular disk health checks".to_string(),
                    "Monitor system for unauthorized access".to_string(),
                ])
                .with_estimated_fix_time(Duration::from_secs(60))
            }
            Self::OrphanedFile { file_path, category } => {
                let (severity, action, message) = match category {
                    OrphanedFileCategory::Leftover => (
                        DiscrepancySeverity::Medium,
                        RecommendedAction::AutoHeal,
                        format!("Leftover file '{file_path}' from a previous package version. Safe to remove.")
                    ),
                    OrphanedFileCategory::UserCreated => (
                        DiscrepancySeverity::Low,
                        RecommendedAction::Ignore,
                        format!("User-created file '{file_path}' found in package directory. Consider moving to appropriate location.")
                    ),
                    OrphanedFileCategory::Temporary => (
                        DiscrepancySeverity::Low,
                        RecommendedAction::AutoHeal,
                        format!("Temporary file '{file_path}' should be cleaned up.")
                    ),
                    OrphanedFileCategory::System => (
                        DiscrepancySeverity::Low,
                        RecommendedAction::Ignore,
                        format!("System file '{file_path}' detected in package directory. Preserving.")
                    ),
                    OrphanedFileCategory::Unknown => (
                        DiscrepancySeverity::Medium,
                        RecommendedAction::UserConfirmation,
                        format!("Unknown file '{file_path}' found in package directory. Manual review recommended.")
                    ),
                };

                let mut context = DiscrepancyContext::new(
                    severity,
                    action,
                    message,
                    format!("Orphaned file: {file_path} (category: {category:?})"),
                );

                match category {
                    OrphanedFileCategory::UserCreated => {
                        context = context
                            .with_manual_steps(vec![
                                format!("Review file contents: cat '{}'", file_path),
                                "Move to appropriate user directory if needed".to_string(),
                                "Add to .gitignore if this is a project directory".to_string(),
                            ])
                            .with_prevention_tips(vec![
                                "Store user files outside package directories".to_string(),
                                "Use designated config/data directories".to_string(),
                            ]);
                    }
                    OrphanedFileCategory::Unknown => {
                        context = context
                            .with_manual_steps(vec![
                                format!("Examine file: file '{}'", file_path),
                                format!("Check file contents: head '{}'", file_path),
                                "Determine if file is needed".to_string(),
                                "Remove if safe, or move to appropriate location".to_string(),
                            ])
                            .with_prevention_tips(vec![
                                "Investigate source of unknown files".to_string(),
                                "Review recent system changes".to_string(),
                            ]);
                    }
                    _ => {}
                }

                context.with_estimated_fix_time(Duration::from_secs(15))
            }
            Self::MissingVenv { package_name, package_version, venv_path } => {
                DiscrepancyContext::new(
                    DiscrepancySeverity::High,
                    RecommendedAction::AutoHeal,
                    format!(
                        "Python virtual environment missing for package '{package_name}' v{package_version} at '{venv_path}'. Python packages may not function correctly."
                    ),
                    format!("Expected virtual environment {venv_path} for {package_name}:{package_version} not found"),
                )
                .with_manual_steps(vec![
                    format!("Recreate virtual environment: python -m venv '{}'", venv_path),
                    format!("Reinstall Python package: sps2 install {}:{}", package_name, package_version),
                    "Verify Python packages: sps2 verify --scope python".to_string(),
                ])
                .with_prevention_tips(vec![
                    "Avoid manually deleting virtual environments".to_string(),
                    "Use sps2 for Python package management".to_string(),
                    "Regular system verification catches venv issues early".to_string(),
                ])
                .with_estimated_fix_time(Duration::from_secs(120))
            }
            Self::MissingPackageContent { package_name, package_version } => {
                DiscrepancyContext::new(
                    DiscrepancySeverity::Critical,
                    RecommendedAction::UserConfirmation,
                    format!(
                        "Package content missing from store for '{package_name}' v{package_version}. Package files cannot be verified."
                    ),
                    format!("Package {package_name}:{package_version} store content is missing"),
                )
                .with_manual_steps(vec![
                    format!("Reinstall package: sps2 install {}:{}", package_name, package_version),
                    "Or remove the package if no longer needed: sps2 remove <package>".to_string(),
                ])
                .with_prevention_tips(vec![
                    "Avoid manually modifying the package store".to_string(),
                    "Use sps2 commands for all package operations".to_string(),
                ])
                .with_estimated_fix_time(Duration::from_secs(60))
            }
            Self::UnsupportedSpecialFile { package_name, package_version, file_path, file_type } => {
                DiscrepancyContext::new(
                    DiscrepancySeverity::Low,
                    RecommendedAction::Ignore,
                    format!(
                        "Special file '{}' ({}) from package '{}' v{} cannot be fully verified. This is expected behavior.",
                        file_path, file_type.description(), package_name, package_version
                    ),
                    format!("Special file {file_path} of type {} detected", file_type.description()),
                )
                .with_manual_steps(vec![
                    "No action required - special files are tracked but not content-verified".to_string(),
                    format!("Verify package integrity: sps2 verify {}:{}", package_name, package_version),
                ])
                .with_prevention_tips(vec![
                    "This is normal behavior for device files, sockets, and FIFOs".to_string(),
                ])
                .with_estimated_fix_time(Duration::from_secs(0))
            }
        }
    }

    /// Get the severity level for this discrepancy
    #[must_use]
    pub fn severity(&self) -> DiscrepancySeverity {
        self.user_context().severity
    }

    /// Get the recommended action for this discrepancy
    #[must_use]
    pub fn recommended_action(&self) -> RecommendedAction {
        self.user_context().recommended_action
    }

    /// Check if this discrepancy can be automatically healed
    #[must_use]
    pub fn can_auto_heal(&self) -> bool {
        matches!(self.recommended_action(), RecommendedAction::AutoHeal)
    }

    /// Check if this discrepancy requires user confirmation before healing
    #[must_use]
    pub fn requires_confirmation(&self) -> bool {
        matches!(
            self.recommended_action(),
            RecommendedAction::UserConfirmation
        )
    }

    /// Get a short, user-friendly description of the discrepancy
    #[must_use]
    pub fn short_description(&self) -> String {
        match self {
            Self::MissingFile { file_path, .. } => format!("Missing file: {file_path}"),
            Self::TypeMismatch {
                file_path,
                expected_directory,
                ..
            } => {
                let expected = if *expected_directory {
                    "directory"
                } else {
                    "file"
                };
                format!("Wrong type for {file_path}: expected {expected}")
            }
            Self::CorruptedFile { file_path, .. } => format!("Corrupted file: {file_path}"),
            Self::OrphanedFile {
                file_path,
                category,
            } => format!("Orphaned {category:?}: {file_path}"),
            Self::MissingVenv { venv_path, .. } => {
                format!("Missing virtual environment: {venv_path}")
            }
            Self::MissingPackageContent {
                package_name,
                package_version,
            } => {
                format!("Missing package content: {package_name}:{package_version}")
            }
            Self::UnsupportedSpecialFile {
                file_path,
                file_type,
                ..
            } => {
                format!("Special file ({}): {}", file_type.description(), file_path)
            }
        }
    }

    /// Get the affected file path for this discrepancy
    #[must_use]
    pub fn file_path(&self) -> &str {
        match self {
            Self::MissingFile { file_path, .. }
            | Self::TypeMismatch { file_path, .. }
            | Self::CorruptedFile { file_path, .. }
            | Self::OrphanedFile { file_path, .. }
            | Self::UnsupportedSpecialFile { file_path, .. } => file_path,
            Self::MissingVenv { venv_path, .. } => venv_path,
            Self::MissingPackageContent { .. } => "",
        }
    }

    /// Get the package name for this discrepancy (if applicable)
    #[must_use]
    pub fn package_name(&self) -> Option<&str> {
        match self {
            Self::MissingFile { package_name, .. }
            | Self::TypeMismatch { package_name, .. }
            | Self::CorruptedFile { package_name, .. }
            | Self::MissingVenv { package_name, .. }
            | Self::MissingPackageContent { package_name, .. }
            | Self::UnsupportedSpecialFile { package_name, .. } => Some(package_name),
            Self::OrphanedFile { .. } => None,
        }
    }

    /// Get the package version for this discrepancy (if applicable)
    #[must_use]
    pub fn package_version(&self) -> Option<&str> {
        match self {
            Self::MissingFile {
                package_version, ..
            }
            | Self::TypeMismatch {
                package_version, ..
            }
            | Self::CorruptedFile {
                package_version, ..
            }
            | Self::MissingVenv {
                package_version, ..
            }
            | Self::MissingPackageContent {
                package_version, ..
            }
            | Self::UnsupportedSpecialFile {
                package_version, ..
            } => Some(package_version),
            Self::OrphanedFile { .. } => None,
        }
    }
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
    /// Cache hit rate as a fraction between 0.0 and 1.0
    pub cache_hit_rate: f64,
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
            cache_hit_rate: 0.0,
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
            cache_hit_rate: 0.0,
        }
    }

    /// Create a new verification result with coverage information and cache hit rate
    #[must_use]
    pub fn with_coverage_and_cache(
        state_id: Uuid,
        discrepancies: Vec<Discrepancy>,
        duration_ms: u64,
        coverage: VerificationCoverage,
        cache_hit_rate: f64,
    ) -> Self {
        let is_valid = discrepancies.is_empty();
        Self {
            state_id,
            discrepancies,
            is_valid,
            duration_ms,
            coverage: Some(coverage),
            cache_hit_rate,
        }
    }
}

/// Context for verification operations to reduce argument count
pub struct VerificationContext<'a> {
    /// State manager for database operations
    pub state_manager: &'a sps2_state::StateManager,
    /// Package store for content verification
    pub store: &'a sps2_store::PackageStore,
    /// Verification level
    pub level: VerificationLevel,
    /// Current state ID being verified
    pub state_id: &'a uuid::Uuid,
    /// Live directory path
    pub live_path: &'a Path,
    /// Guard configuration for policies and settings
    pub guard_config: &'a GuardConfig,
    /// Event sender for logging (optional)
    pub tx: Option<&'a sps2_events::EventSender>,
    /// Verification scope for filtering
    pub scope: &'a VerificationScope,
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

impl<'a> EventEmitter for HealingContext<'a> {
    fn event_sender(&self) -> Option<&EventSender> {
        Some(self.tx)
    }
}

/// Policy for handling symlinks during verification
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SymlinkPolicy {
    /// Verify symlinks strictly - fail on any symlink issues
    Strict,
    /// Be lenient with symlinks - log issues but don't fail verification
    /// Useful for bootstrap directories like /opt/pm/live/bin
    Lenient,
    /// Ignore symlinks entirely - skip symlink verification
    Ignore,
}

impl Default for SymlinkPolicy {
    fn default() -> Self {
        Self::Strict
    }
}

/// Performance configuration for guard operations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PerformanceConfig {
    /// Use progressive verification (Quick → Standard → Full as needed)
    pub progressive_verification: bool,
    /// Maximum number of concurrent verification tasks
    pub max_concurrent_tasks: usize,
    /// Timeout for individual verification operations
    pub verification_timeout: Duration,
    /// Number of files to process in each chunk
    pub file_chunk_size: usize,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            progressive_verification: true,
            max_concurrent_tasks: 8,
            verification_timeout: Duration::from_secs(300), // 5 minutes
            file_chunk_size: 100,                           // Process 100 files per chunk
        }
    }
}

/// Comprehensive guard configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuardConfig {
    /// Verification level to use
    pub verification_level: VerificationLevel,
    /// How to handle discrepancies
    pub discrepancy_handling: DiscrepancyHandling,
    /// Policy for handling symlinks
    pub symlink_policy: SymlinkPolicy,
    /// Performance configuration
    pub performance: PerformanceConfig,
    /// Directories where symlinks should be handled leniently
    pub lenient_symlink_directories: Vec<PathBuf>,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            verification_level: VerificationLevel::Standard,
            discrepancy_handling: DiscrepancyHandling::AutoHeal,
            symlink_policy: SymlinkPolicy::Lenient,
            performance: PerformanceConfig::default(),
            lenient_symlink_directories: vec![
                PathBuf::from("/opt/pm/live/bin"),
                PathBuf::from("/opt/pm/live/sbin"),
            ],
        }
    }
}

impl GuardConfig {
    /// Check if should fail on discrepancy (for backward compatibility)
    pub fn should_fail_on_discrepancy(&self) -> bool {
        matches!(
            self.discrepancy_handling,
            DiscrepancyHandling::FailFast | DiscrepancyHandling::AutoHealOrFail
        )
    }

    /// Check if should auto-heal (for backward compatibility)
    pub fn should_auto_heal(&self) -> bool {
        matches!(
            self.discrepancy_handling,
            DiscrepancyHandling::AutoHeal | DiscrepancyHandling::AutoHealOrFail
        )
    }
}

/// Type of package operation being performed
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OperationType {
    /// Install new packages
    Install {
        /// Package specifications to install
        package_specs: Vec<String>,
    },
    /// Remove existing packages
    Uninstall {
        /// Package names to remove
        package_names: Vec<String>,
    },
    /// Upgrade packages to latest versions (ignore upper bounds)
    Upgrade {
        /// Package names to upgrade (empty = all packages)
        package_names: Vec<String>,
    },
    /// Update packages respecting constraints
    Update {
        /// Package names to update (empty = all packages)
        package_names: Vec<String>,
    },
    /// Build package from recipe (only in scope if install is triggered)
    Build {
        /// Recipe path
        recipe_path: PathBuf,
    },
    /// Rollback to previous state
    Rollback {
        /// Target state ID (None = previous state)
        target_state_id: Option<uuid::Uuid>,
    },
    /// Clean up orphaned data
    Cleanup,
    /// Verify system state
    Verify {
        /// Scope for verification
        scope: VerificationScope,
    },
}

impl OperationType {
    /// Get the impact level of this operation for verification planning
    #[must_use]
    pub fn impact_level(&self) -> OperationImpact {
        match self {
            Self::Install { .. } | Self::Uninstall { .. } | Self::Rollback { .. } => {
                OperationImpact::High
            }
            Self::Upgrade { package_names } | Self::Update { package_names } => {
                if package_names.is_empty() {
                    OperationImpact::High // All packages
                } else {
                    OperationImpact::Medium // Specific packages
                }
            }
            Self::Build { .. } => {
                // Build itself has low impact - only install afterward has impact
                OperationImpact::Low
            }
            Self::Cleanup => OperationImpact::Medium,
            Self::Verify { .. } => OperationImpact::Low,
        }
    }

    /// Get the recommended verification level for this operation
    #[must_use]
    pub fn recommended_verification_level(&self) -> VerificationLevel {
        match self.impact_level() {
            OperationImpact::High => VerificationLevel::Standard,
            OperationImpact::Medium => VerificationLevel::Standard,
            OperationImpact::Low => VerificationLevel::Quick,
        }
    }

    /// Check if this operation modifies the system state
    #[must_use]
    pub fn modifies_state(&self) -> bool {
        match self {
            Self::Install { .. }
            | Self::Uninstall { .. }
            | Self::Upgrade { .. }
            | Self::Update { .. }
            | Self::Rollback { .. }
            | Self::Cleanup => true,
            Self::Build { .. } | Self::Verify { .. } => false,
        }
    }
}

/// Impact level of an operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationImpact {
    /// High impact - affects many packages or system-wide changes
    High,
    /// Medium impact - affects specific packages or moderate changes
    Medium,
    /// Low impact - minimal or no changes
    Low,
}

/// Result of an operation for post-verification scoping
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OperationResult {
    /// Packages that were installed
    pub installed: Vec<PackageChange>,
    /// Packages that were updated
    pub updated: Vec<PackageChange>,
    /// Packages that were removed
    pub removed: Vec<PackageChange>,
    /// New state ID after operation
    pub state_id: uuid::Uuid,
    /// Operation duration in milliseconds
    pub duration_ms: u64,
    /// Directories that were modified
    pub modified_directories: Vec<PathBuf>,
    /// Whether install was triggered during build operation
    pub install_triggered: bool,
}

/// Information about a package change
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackageChange {
    /// Package name
    pub name: String,
    /// Previous version (None for new installs)
    pub from_version: Option<String>,
    /// New version (None for removals)
    pub to_version: Option<String>,
    /// Package size in bytes
    pub size: Option<u64>,
}

/// Derive verification scope for pre-operation verification
///
/// This determines what should be verified before an operation starts
/// to ensure the system is in a consistent state.
pub fn derive_pre_operation_scope(operation: &OperationType) -> VerificationScope {
    match operation {
        OperationType::Install { package_specs } => {
            // For install, verify that we're not overwriting existing packages
            // and that the system is clean for dependency resolution
            if package_specs.len() == 1 {
                // Single package install - can be scoped to related directories
                VerificationScope::Directory {
                    path: PathBuf::from("/opt/pm/live"),
                }
            } else {
                // Multiple packages - full verification for safety
                VerificationScope::Full
            }
        }
        OperationType::Uninstall { package_names } => {
            // Verify the packages we're about to remove actually exist and are consistent
            let packages = package_names
                .iter()
                .map(|name| (name.clone(), "*".to_string())) // Version will be resolved
                .collect();
            VerificationScope::Packages { packages }
        }
        OperationType::Upgrade { package_names } | OperationType::Update { package_names } => {
            if package_names.is_empty() {
                // All packages - full verification
                VerificationScope::Full
            } else {
                // Specific packages - verify only those
                let packages = package_names
                    .iter()
                    .map(|name| (name.clone(), "*".to_string()))
                    .collect();
                VerificationScope::Packages { packages }
            }
        }
        OperationType::Build { .. } => {
            // Build operations don't modify live system - no pre-verification needed
            VerificationScope::Directory {
                path: PathBuf::from("/tmp"), // Minimal scope - just to satisfy the interface
            }
        }
        OperationType::Rollback { .. } => {
            // Rollback affects everything - full verification
            VerificationScope::Full
        }
        OperationType::Cleanup => {
            // Cleanup affects orphaned data - verify store and state consistency
            VerificationScope::Full
        }
        OperationType::Verify { scope } => {
            // For verify operations, use the specified scope
            scope.clone()
        }
    }
}

/// Derive verification scope for post-operation verification
///
/// This determines what should be verified after an operation completes
/// to ensure the operation succeeded and the system is consistent.
pub fn derive_post_operation_scope(
    operation: &OperationType,
    result: &OperationResult,
) -> VerificationScope {
    // Collect all affected packages from the operation result
    let mut affected_packages = Vec::new();
    let mut modified_directories = result.modified_directories.clone();

    // Add packages that were installed
    for package in &result.installed {
        if let Some(version) = &package.to_version {
            affected_packages.push((package.name.clone(), version.clone()));
        }
    }

    // Add packages that were updated
    for package in &result.updated {
        if let Some(version) = &package.to_version {
            affected_packages.push((package.name.clone(), version.clone()));
        }
    }

    // For removed packages, verify they're actually gone
    let removed_packages: Vec<_> = result
        .removed
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                p.from_version.clone().unwrap_or_else(|| "*".to_string()),
            )
        })
        .collect();

    match operation {
        OperationType::Install { .. } => {
            // Verify newly installed packages and their dependencies
            if affected_packages.is_empty() {
                VerificationScope::Directory {
                    path: PathBuf::from("/opt/pm/live"),
                }
            } else {
                VerificationScope::Packages {
                    packages: affected_packages,
                }
            }
        }
        OperationType::Uninstall { .. } => {
            // Verify packages are removed and check for orphaned files
            if modified_directories.is_empty() {
                modified_directories.push(PathBuf::from("/opt/pm/live"));
            }

            if removed_packages.is_empty() && affected_packages.is_empty() {
                VerificationScope::Full
            } else {
                VerificationScope::Mixed {
                    packages: affected_packages,
                    directories: modified_directories,
                }
            }
        }
        OperationType::Upgrade { package_names } | OperationType::Update { package_names } => {
            if package_names.is_empty() || affected_packages.len() > 10 {
                // Many packages updated - full verification
                VerificationScope::Full
            } else {
                // Specific packages updated - verify only those
                VerificationScope::Packages {
                    packages: affected_packages,
                }
            }
        }
        OperationType::Build { .. } => {
            if result.install_triggered {
                // Install was triggered during build - verify the installed package
                // This follows the same logic as Install operations
                if affected_packages.is_empty() {
                    VerificationScope::Directory {
                        path: PathBuf::from("/opt/pm/live"),
                    }
                } else {
                    VerificationScope::Packages {
                        packages: affected_packages,
                    }
                }
            } else {
                // Just built without install - minimal verification (build doesn't modify live system)
                VerificationScope::Directory {
                    path: PathBuf::from("/tmp"), // Minimal scope
                }
            }
        }
        OperationType::Rollback { .. } => {
            // Rollback changes everything - full verification required
            VerificationScope::Full
        }
        OperationType::Cleanup => {
            // Cleanup may affect many areas - full verification
            VerificationScope::Full
        }
        OperationType::Verify { .. } => {
            // Verify operations don't change state - no post-verification needed
            VerificationScope::Directory {
                path: PathBuf::from("/tmp"), // Minimal scope
            }
        }
    }
}

/// Smart scope selection based on operation impact and system state
///
/// This function provides intelligent scope selection that balances
/// verification thoroughness with performance.
pub fn select_smart_scope(
    operation: &OperationType,
    pre_operation: bool,
    system_package_count: usize,
) -> VerificationScope {
    let base_scope = if pre_operation {
        derive_pre_operation_scope(operation)
    } else {
        // For post-operation, we need the result, so fall back to operation-based logic
        match operation {
            OperationType::Install { package_specs } => {
                if package_specs.len() == 1 {
                    VerificationScope::Directory {
                        path: PathBuf::from("/opt/pm/live"),
                    }
                } else {
                    VerificationScope::Full
                }
            }
            OperationType::Uninstall { package_names } => {
                let packages = package_names
                    .iter()
                    .map(|name| (name.clone(), "*".to_string()))
                    .collect();
                VerificationScope::Packages { packages }
            }
            OperationType::Build { .. } => {
                // Build operations have minimal scope - no state modification
                VerificationScope::Directory {
                    path: PathBuf::from("/tmp"),
                }
            }
            _ => derive_pre_operation_scope(operation),
        }
    };

    // Apply performance optimizations based on system size
    match base_scope {
        VerificationScope::Full if system_package_count > 100 => {
            // For large systems, consider using directory-based verification
            // for medium-impact operations
            if operation.impact_level() == OperationImpact::Medium {
                VerificationScope::Directory {
                    path: PathBuf::from("/opt/pm/live"),
                }
            } else {
                base_scope
            }
        }
        _ => base_scope,
    }
}

// Configuration conversions from sps2-config types

/// Convert config SymlinkPolicyConfig to guard SymlinkPolicy
impl From<sps2_config::SymlinkPolicyConfig> for SymlinkPolicy {
    fn from(config: sps2_config::SymlinkPolicyConfig) -> Self {
        match config {
            sps2_config::SymlinkPolicyConfig::Strict => Self::Strict,
            sps2_config::SymlinkPolicyConfig::LenientBootstrap => Self::Lenient, // Maps to existing Lenient
            sps2_config::SymlinkPolicyConfig::LenientAll => Self::Lenient,
            sps2_config::SymlinkPolicyConfig::Ignore => Self::Ignore,
        }
    }
}

/// Convert config PerformanceConfigToml to guard PerformanceConfig
impl From<&sps2_config::PerformanceConfigToml> for PerformanceConfig {
    fn from(config: &sps2_config::PerformanceConfigToml) -> Self {
        Self {
            progressive_verification: config.progressive_verification,
            max_concurrent_tasks: config.max_concurrent_tasks,
            verification_timeout: Duration::from_secs(config.verification_timeout_seconds),
            file_chunk_size: 100, // Use default chunk size
        }
    }
}

/// Convert config VerificationConfig to guard GuardConfig
impl From<&sps2_config::VerificationConfig> for GuardConfig {
    fn from(config: &sps2_config::VerificationConfig) -> Self {
        // Parse verification level
        let verification_level = match config.level.as_str() {
            "quick" => VerificationLevel::Quick,
            "full" => VerificationLevel::Full,
            _ => VerificationLevel::Standard,
        };

        // Determine symlink policy with intelligent defaults
        let symlink_policy = match config.guard.symlink_policy {
            sps2_config::SymlinkPolicyConfig::LenientBootstrap => {
                // For backward compatibility, use lenient policy for bootstrap directories
                SymlinkPolicy::Lenient
            }
            other => other.into(),
        };

        Self {
            verification_level,
            discrepancy_handling: config.discrepancy_handling,
            symlink_policy,
            performance: (&config.performance).into(),
            lenient_symlink_directories: config.guard.lenient_symlink_directories.clone(),
        }
    }
}

// Conversions for top-level guard configuration

/// Convert config GuardSymlinkPolicy to guard SymlinkPolicy
impl From<sps2_config::GuardSymlinkPolicy> for SymlinkPolicy {
    fn from(config: sps2_config::GuardSymlinkPolicy) -> Self {
        match config {
            sps2_config::GuardSymlinkPolicy::Strict => Self::Strict,
            sps2_config::GuardSymlinkPolicy::Lenient => Self::Lenient,
            sps2_config::GuardSymlinkPolicy::Ignore => Self::Ignore,
        }
    }
}

/// Convert config GuardPerformanceConfig to guard PerformanceConfig
impl From<&sps2_config::GuardPerformanceConfig> for PerformanceConfig {
    fn from(config: &sps2_config::GuardPerformanceConfig) -> Self {
        Self {
            progressive_verification: config.progressive_verification,
            max_concurrent_tasks: config.max_concurrent_tasks,
            verification_timeout: Duration::from_secs(config.verification_timeout_seconds),
            file_chunk_size: 100, // Use default chunk size
        }
    }
}

/// Convert config GuardConfiguration to guard GuardConfig
impl From<&sps2_config::GuardConfiguration> for GuardConfig {
    fn from(config: &sps2_config::GuardConfiguration) -> Self {
        // Parse verification level
        let verification_level = match config.verification_level.as_str() {
            "quick" => VerificationLevel::Quick,
            "full" => VerificationLevel::Full,
            _ => VerificationLevel::Standard,
        };

        Self {
            verification_level,
            discrepancy_handling: config.discrepancy_handling,
            symlink_policy: config.symlink_policy.into(),
            performance: (&config.performance).into(),
            lenient_symlink_directories: config
                .lenient_symlink_directories
                .iter()
                .map(|dir_config| dir_config.path.clone())
                .collect(),
        }
    }
}

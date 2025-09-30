use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::time::Duration;

use super::FailureContext;

/// Generic lifecycle stages for simple operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStage {
    Started,
    Completed,
    Failed,
}

/// Domain identifier for lifecycle events
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleDomain {
    Acquisition,
    Download,
    Install,
    Resolver,
    Repo,
    Uninstall,
    Update,
}

// ============================================================================
// Domain-specific context structures
// ============================================================================

/// Context for acquisition events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquisitionContext {
    pub package: String,
    pub version: Version,
    pub source: LifecycleAcquisitionSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Source of package acquisition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAcquisitionSource {
    Remote { url: String, mirror_priority: u8 },
    StoreCache { hash: String },
}

/// Context for download events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadContext {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_downloaded: Option<u64>,
}

/// Context for install events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallContext {
    pub package: String,
    pub version: Version,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_installed: Option<usize>,
}

/// Context for resolver events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverContext {
    // Started fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_targets: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_targets: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_targets: Option<usize>,
    // Completed fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_packages: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded_packages: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reused_packages: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    // Failed fields
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicting_packages: Vec<String>,
}

/// Context for repo sync events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages_updated: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_transferred: Option<u64>,
}

/// Context for uninstall events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_removed: Option<usize>,
}

/// Context for update events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateContext {
    pub operation: LifecycleUpdateOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_targets: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<Vec<LifecycleUpdateResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_difference: Option<i64>,
}

/// Types of update operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleUpdateOperation {
    Update,
    Upgrade,
    Downgrade,
    Reinstall,
}

/// Package update types based on semantic versioning
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePackageUpdateType {
    Patch,
    Minor,
    Major,
    PreRelease,
}

/// Update result for completed package updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleUpdateResult {
    pub package: String,
    pub from_version: Version,
    pub to_version: Version,
    pub update_type: LifecyclePackageUpdateType,
    pub duration: Duration,
    pub size_change: i64,
}

// ============================================================================
// Generic lifecycle event structure
// ============================================================================

/// Generic lifecycle event that consolidates simple Started/Completed/Failed patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LifecycleEvent {
    Acquisition {
        stage: LifecycleStage,
        context: AcquisitionContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<FailureContext>,
    },
    Download {
        stage: LifecycleStage,
        context: DownloadContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<FailureContext>,
    },
    Install {
        stage: LifecycleStage,
        context: InstallContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<FailureContext>,
    },
    Resolver {
        stage: LifecycleStage,
        context: ResolverContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<FailureContext>,
    },
    Repo {
        stage: LifecycleStage,
        context: RepoContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<FailureContext>,
    },
    Uninstall {
        stage: LifecycleStage,
        context: UninstallContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<FailureContext>,
    },
    Update {
        stage: LifecycleStage,
        context: UpdateContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<FailureContext>,
    },
}

// ============================================================================
// Helper methods for ergonomic event creation
// ============================================================================

impl LifecycleEvent {
    // Acquisition helpers
    /// Create an acquisition started event
    #[must_use]
    pub fn acquisition_started(
        package: String,
        version: Version,
        source: LifecycleAcquisitionSource,
    ) -> Self {
        Self::Acquisition {
            stage: LifecycleStage::Started,
            context: AcquisitionContext {
                package,
                version,
                source,
                size: None,
            },
            failure: None,
        }
    }

    /// Create an acquisition completed event
    #[must_use]
    pub fn acquisition_completed(
        package: String,
        version: Version,
        source: LifecycleAcquisitionSource,
        size: u64,
    ) -> Self {
        Self::Acquisition {
            stage: LifecycleStage::Completed,
            context: AcquisitionContext {
                package,
                version,
                source,
                size: Some(size),
            },
            failure: None,
        }
    }

    /// Create an acquisition failed event
    #[must_use]
    pub fn acquisition_failed(
        package: String,
        version: Version,
        source: LifecycleAcquisitionSource,
        failure: FailureContext,
    ) -> Self {
        Self::Acquisition {
            stage: LifecycleStage::Failed,
            context: AcquisitionContext {
                package,
                version,
                source,
                size: None,
            },
            failure: Some(failure),
        }
    }

    // Download helpers
    /// Create a download started event
    #[must_use]
    pub fn download_started(
        url: String,
        package: Option<String>,
        total_bytes: Option<u64>,
    ) -> Self {
        Self::Download {
            stage: LifecycleStage::Started,
            context: DownloadContext {
                url,
                package,
                total_bytes,
                bytes_downloaded: None,
            },
            failure: None,
        }
    }

    /// Create a download completed event
    #[must_use]
    pub fn download_completed(url: String, package: Option<String>, bytes_downloaded: u64) -> Self {
        Self::Download {
            stage: LifecycleStage::Completed,
            context: DownloadContext {
                url,
                package,
                total_bytes: None,
                bytes_downloaded: Some(bytes_downloaded),
            },
            failure: None,
        }
    }

    /// Create a download failed event
    #[must_use]
    pub fn download_failed(url: String, package: Option<String>, failure: FailureContext) -> Self {
        Self::Download {
            stage: LifecycleStage::Failed,
            context: DownloadContext {
                url,
                package,
                total_bytes: None,
                bytes_downloaded: None,
            },
            failure: Some(failure),
        }
    }

    // Install helpers
    /// Create an install started event
    #[must_use]
    pub fn install_started(package: String, version: Version) -> Self {
        Self::Install {
            stage: LifecycleStage::Started,
            context: InstallContext {
                package,
                version,
                files_installed: None,
            },
            failure: None,
        }
    }

    /// Create an install completed event
    #[must_use]
    pub fn install_completed(package: String, version: Version, files_installed: usize) -> Self {
        Self::Install {
            stage: LifecycleStage::Completed,
            context: InstallContext {
                package,
                version,
                files_installed: Some(files_installed),
            },
            failure: None,
        }
    }

    /// Create an install failed event
    #[must_use]
    pub fn install_failed(package: String, version: Version, failure: FailureContext) -> Self {
        Self::Install {
            stage: LifecycleStage::Failed,
            context: InstallContext {
                package,
                version,
                files_installed: None,
            },
            failure: Some(failure),
        }
    }

    // Resolver helpers
    // Resolver helpers
    /// Create a resolver started event
    #[must_use]
    pub fn resolver_started(
        runtime_targets: usize,
        build_targets: usize,
        local_targets: usize,
    ) -> Self {
        Self::Resolver {
            stage: LifecycleStage::Started,
            context: ResolverContext {
                runtime_targets: Some(runtime_targets),
                build_targets: Some(build_targets),
                local_targets: Some(local_targets),
                total_packages: None,
                downloaded_packages: None,
                reused_packages: None,
                duration_ms: None,
                conflicting_packages: vec![],
            },
            failure: None,
        }
    }

    /// Create a resolver completed event
    #[must_use]
    pub fn resolver_completed(
        total_packages: usize,
        downloaded_packages: usize,
        reused_packages: usize,
        duration_ms: u64,
    ) -> Self {
        Self::Resolver {
            stage: LifecycleStage::Completed,
            context: ResolverContext {
                runtime_targets: None,
                build_targets: None,
                local_targets: None,
                total_packages: Some(total_packages),
                downloaded_packages: Some(downloaded_packages),
                reused_packages: Some(reused_packages),
                duration_ms: Some(duration_ms),
                conflicting_packages: vec![],
            },
            failure: None,
        }
    }

    /// Create a resolver failed event
    #[must_use]
    pub fn resolver_failed(failure: FailureContext, conflicting_packages: Vec<String>) -> Self {
        Self::Resolver {
            stage: LifecycleStage::Failed,
            context: ResolverContext {
                runtime_targets: None,
                build_targets: None,
                local_targets: None,
                total_packages: None,
                downloaded_packages: None,
                reused_packages: None,
                duration_ms: None,
                conflicting_packages,
            },
            failure: Some(failure),
        }
    }

    // Repo helpers
    /// Create a repo sync started event
    #[must_use]
    pub fn repo_sync_started(url: Option<String>) -> Self {
        Self::Repo {
            stage: LifecycleStage::Started,
            context: RepoContext {
                url,
                packages_updated: None,
                duration_ms: None,
                bytes_transferred: None,
            },
            failure: None,
        }
    }

    /// Create a repo sync completed event
    #[must_use]
    pub fn repo_sync_completed(
        packages_updated: usize,
        duration_ms: u64,
        bytes_transferred: u64,
    ) -> Self {
        Self::Repo {
            stage: LifecycleStage::Completed,
            context: RepoContext {
                url: None,
                packages_updated: Some(packages_updated),
                duration_ms: Some(duration_ms),
                bytes_transferred: Some(bytes_transferred),
            },
            failure: None,
        }
    }

    /// Create a repo sync failed event
    #[must_use]
    pub fn repo_sync_failed(url: Option<String>, failure: FailureContext) -> Self {
        Self::Repo {
            stage: LifecycleStage::Failed,
            context: RepoContext {
                url,
                packages_updated: None,
                duration_ms: None,
                bytes_transferred: None,
            },
            failure: Some(failure),
        }
    }

    // Uninstall helpers
    /// Create an uninstall started event
    #[must_use]
    pub fn uninstall_started(package: String, version: Version) -> Self {
        Self::Uninstall {
            stage: LifecycleStage::Started,
            context: UninstallContext {
                package: Some(package),
                version: Some(version),
                files_removed: None,
            },
            failure: None,
        }
    }

    /// Create an uninstall completed event
    #[must_use]
    pub fn uninstall_completed(package: String, version: Version, files_removed: usize) -> Self {
        Self::Uninstall {
            stage: LifecycleStage::Completed,
            context: UninstallContext {
                package: Some(package),
                version: Some(version),
                files_removed: Some(files_removed),
            },
            failure: None,
        }
    }

    /// Create an uninstall failed event
    #[must_use]
    pub fn uninstall_failed(
        package: Option<String>,
        version: Option<Version>,
        failure: FailureContext,
    ) -> Self {
        Self::Uninstall {
            stage: LifecycleStage::Failed,
            context: UninstallContext {
                package,
                version,
                files_removed: None,
            },
            failure: Some(failure),
        }
    }

    // Update helpers
    /// Create an update started event
    #[must_use]
    pub fn update_started(
        operation: LifecycleUpdateOperation,
        requested: Vec<String>,
        total_targets: usize,
    ) -> Self {
        Self::Update {
            stage: LifecycleStage::Started,
            context: UpdateContext {
                operation,
                requested: Some(requested),
                total_targets: Some(total_targets),
                updated: None,
                skipped: None,
                failed: None,
                duration: None,
                size_difference: None,
            },
            failure: None,
        }
    }

    /// Create an update completed event
    #[must_use]
    pub fn update_completed(
        operation: LifecycleUpdateOperation,
        updated: Vec<LifecycleUpdateResult>,
        skipped: usize,
        duration: Duration,
        size_difference: i64,
    ) -> Self {
        Self::Update {
            stage: LifecycleStage::Completed,
            context: UpdateContext {
                operation,
                requested: None,
                total_targets: None,
                updated: Some(updated),
                skipped: Some(skipped),
                failed: None,
                duration: Some(duration),
                size_difference: Some(size_difference),
            },
            failure: None,
        }
    }

    /// Create an update failed event
    #[must_use]
    pub fn update_failed(
        operation: LifecycleUpdateOperation,
        updated: Vec<LifecycleUpdateResult>,
        failed: Vec<String>,
        failure: FailureContext,
    ) -> Self {
        Self::Update {
            stage: LifecycleStage::Failed,
            context: UpdateContext {
                operation,
                requested: None,
                total_targets: None,
                updated: Some(updated),
                skipped: None,
                failed: Some(failed),
                duration: None,
                size_difference: None,
            },
            failure: Some(failure),
        }
    }

    /// Get the domain for this lifecycle event
    #[must_use]
    pub fn domain(&self) -> LifecycleDomain {
        match self {
            Self::Acquisition { .. } => LifecycleDomain::Acquisition,
            Self::Download { .. } => LifecycleDomain::Download,
            Self::Install { .. } => LifecycleDomain::Install,
            Self::Resolver { .. } => LifecycleDomain::Resolver,
            Self::Repo { .. } => LifecycleDomain::Repo,
            Self::Uninstall { .. } => LifecycleDomain::Uninstall,
            Self::Update { .. } => LifecycleDomain::Update,
        }
    }

    /// Get the stage for this lifecycle event
    #[must_use]
    pub fn stage(&self) -> &LifecycleStage {
        match self {
            Self::Acquisition { stage, .. }
            | Self::Download { stage, .. }
            | Self::Install { stage, .. }
            | Self::Resolver { stage, .. }
            | Self::Repo { stage, .. }
            | Self::Uninstall { stage, .. }
            | Self::Update { stage, .. } => stage,
        }
    }

    /// Get the failure context if this is a failed event
    #[must_use]
    pub fn failure(&self) -> Option<&FailureContext> {
        match self {
            Self::Acquisition { failure, .. }
            | Self::Download { failure, .. }
            | Self::Install { failure, .. }
            | Self::Resolver { failure, .. }
            | Self::Repo { failure, .. }
            | Self::Uninstall { failure, .. }
            | Self::Update { failure, .. } => failure.as_ref(),
        }
    }
}

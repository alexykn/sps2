//! Types for operations and results

use serde::{Deserialize, Serialize};
use spsv2_types::{PackageSpec, Version};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Operation report for complex operations
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpReport {
    /// Operation type
    pub operation: String,
    /// Whether the operation succeeded
    pub success: bool,
    /// Summary message
    pub summary: String,
    /// Detailed changes
    pub changes: Vec<OpChange>,
    /// Execution time in milliseconds
    pub duration_ms: u64,
}

impl OpReport {
    /// Create success report
    #[must_use]
    pub fn success(
        operation: String,
        summary: String,
        changes: Vec<OpChange>,
        duration_ms: u64,
    ) -> Self {
        Self {
            operation,
            success: true,
            summary,
            changes,
            duration_ms,
        }
    }

    /// Create failure report
    #[must_use]
    pub fn failure(operation: String, summary: String, duration_ms: u64) -> Self {
        Self {
            operation,
            success: false,
            summary,
            changes: Vec::new(),
            duration_ms,
        }
    }
}

/// Operation change
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpChange {
    /// Change type
    pub change_type: ChangeType,
    /// Package name
    pub package: String,
    /// Old version (for updates/removals)
    pub old_version: Option<Version>,
    /// New version (for installs/updates)
    pub new_version: Option<Version>,
}

/// Type of operation change
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    /// Package was installed
    Install,
    /// Package was updated
    Update,
    /// Package was removed
    Remove,
    /// Package was downgraded
    Downgrade,
}

/// Package information for display
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageInfo {
    /// Package name
    pub name: String,
    /// Installed version
    pub version: Option<Version>,
    /// Available version
    pub available_version: Option<Version>,
    /// Description
    pub description: Option<String>,
    /// Homepage URL
    pub homepage: Option<String>,
    /// License
    pub license: Option<String>,
    /// Installation status
    pub status: PackageStatus,
    /// Dependencies
    pub dependencies: Vec<String>,
    /// Size on disk (bytes)
    pub size: Option<u64>,
}

/// Package installation status
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageStatus {
    /// Not installed
    Available,
    /// Installed and up to date
    Installed,
    /// Installed but update available
    Outdated,
    /// Installed from local file
    Local,
}

/// Search result
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    /// Package name
    pub name: String,
    /// Latest version
    pub version: Version,
    /// Description
    pub description: Option<String>,
    /// Whether package is installed
    pub installed: bool,
}

/// State information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateInfo {
    /// State ID
    pub id: Uuid,
    /// Creation timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Parent state ID
    pub parent_id: Option<Uuid>,
    /// Whether this is the current state
    pub current: bool,
    /// Number of packages
    pub package_count: usize,
    /// Summary of changes from parent
    pub changes: Vec<OpChange>,
}

/// Health check results
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthCheck {
    /// Overall health status
    pub healthy: bool,
    /// Component checks
    pub components: HashMap<String, ComponentHealth>,
    /// Issues found
    pub issues: Vec<HealthIssue>,
}

impl HealthCheck {
    /// Check if system is healthy
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.healthy
    }
}

/// Component health status
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentHealth {
    /// Component name
    pub name: String,
    /// Health status
    pub status: HealthStatus,
    /// Status message
    pub message: String,
    /// Check duration in milliseconds
    pub check_duration_ms: u64,
}

/// Health status
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// Component is healthy
    Healthy,
    /// Component has warnings
    Warning,
    /// Component is unhealthy
    Error,
}

/// Health issue
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthIssue {
    /// Component where issue was found
    pub component: String,
    /// Severity level
    pub severity: IssueSeverity,
    /// Issue description
    pub description: String,
    /// Suggested fix
    pub suggestion: Option<String>,
}

/// Issue severity
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    /// Low severity
    Low,
    /// Medium severity
    Medium,
    /// High severity
    High,
    /// Critical severity
    Critical,
}

/// Install request type
#[derive(Clone, Debug)]
pub enum InstallRequest {
    /// Install from repository
    Remote(PackageSpec),
    /// Install from local file
    LocalFile(PathBuf),
}

/// Installation report
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallReport {
    /// Packages that were installed
    pub installed: Vec<PackageChange>,
    /// Packages that were updated
    pub updated: Vec<PackageChange>,
    /// Packages that were removed
    pub removed: Vec<PackageChange>,
    /// New state ID
    pub state_id: Uuid,
    /// Total execution time
    pub duration_ms: u64,
}

/// Package change for reports
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageChange {
    /// Package name
    pub name: String,
    /// Previous version
    pub from_version: Option<Version>,
    /// New version
    pub to_version: Option<Version>,
    /// Size in bytes
    pub size: Option<u64>,
}

/// Build report
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildReport {
    /// Package that was built
    pub package: String,
    /// Version that was built
    pub version: Version,
    /// Output file path
    pub output_path: PathBuf,
    /// Build duration
    pub duration_ms: u64,
    /// Whether SBOM was generated
    pub sbom_generated: bool,
}

/// Vulnerability database statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VulnDbStats {
    /// Total number of vulnerabilities
    pub vulnerability_count: usize,
    /// Last update timestamp
    pub last_updated: Option<chrono::DateTime<chrono::Utc>>,
    /// Database size in bytes
    pub database_size: u64,
    /// Breakdown by severity
    pub severity_breakdown: HashMap<String, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_op_report() {
        let changes = vec![OpChange {
            change_type: ChangeType::Install,
            package: "curl".to_string(),
            old_version: None,
            new_version: Some(Version::parse("8.5.0").unwrap()),
        }];

        let report = OpReport::success(
            "install".to_string(),
            "Installed 1 package".to_string(),
            changes,
            1500,
        );

        assert!(report.success);
        assert_eq!(report.changes.len(), 1);
        assert_eq!(report.duration_ms, 1500);
    }

    #[test]
    fn test_package_info() {
        let info = PackageInfo {
            name: "curl".to_string(),
            version: Some(Version::parse("8.5.0").unwrap()),
            available_version: Some(Version::parse("8.6.0").unwrap()),
            description: Some("HTTP client".to_string()),
            homepage: Some("https://curl.se".to_string()),
            license: Some("MIT".to_string()),
            status: PackageStatus::Outdated,
            dependencies: vec!["openssl>=3.0.0".to_string()],
            size: Some(1_024_000),
        };

        assert_eq!(info.name, "curl");
        assert!(matches!(info.status, PackageStatus::Outdated));
        assert_eq!(info.dependencies.len(), 1);
    }

    #[test]
    fn test_health_check() {
        let mut components = HashMap::new();
        components.insert(
            "store".to_string(),
            ComponentHealth {
                name: "store".to_string(),
                status: HealthStatus::Healthy,
                message: "All checks passed".to_string(),
                check_duration_ms: 50,
            },
        );

        let health_check = HealthCheck {
            healthy: true,
            components,
            issues: Vec::new(),
        };

        assert!(health_check.is_healthy());
        assert_eq!(health_check.components.len(), 1);
        assert!(health_check.issues.is_empty());
    }

    #[test]
    fn test_install_request() {
        let remote = InstallRequest::Remote(PackageSpec::parse("curl>=8.0.0").unwrap());
        let local = InstallRequest::LocalFile(PathBuf::from("/path/to/package.sp"));

        match remote {
            InstallRequest::Remote(spec) => assert_eq!(spec.name, "curl"),
            InstallRequest::LocalFile(_) => panic!("Expected remote request"),
        }

        match local {
            InstallRequest::LocalFile(path) => {
                assert!(path.display().to_string().ends_with("package.sp"))
            }
            InstallRequest::Remote(_) => panic!("Expected local file request"),
        }
    }
}

//! Types for operations and results

use serde::{Deserialize, Serialize};
use sps2_events::HealthStatus;
use sps2_types::{OpChange, PackageSpec};
use std::collections::HashMap;
use std::path::PathBuf;
// No longer needed - uuid::Uuid imported from sps2_types

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

// OpChange and ChangeType are now imported from sps2_types

// PackageInfo is now imported from sps2_types

// PackageStatus is now imported from sps2_types

// SearchResult is now imported from sps2_types

// StateInfo is now imported from sps2_types

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

// HealthStatus is now imported from sps2_events

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

// InstallReport is now imported from sps2_types

// PackageChange is now imported from sps2_types

// BuildReport is now imported from sps2_types

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
    use sps2_types::{ChangeType, PackageInfo, PackageStatus, Version};

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
            arch: None,
            installed: true,
        };

        assert_eq!(info.name, "curl");
        assert!(matches!(info.status, PackageStatus::Outdated));
        assert_eq!(info.dependencies.len(), 1);
        assert!(info.installed);
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
                assert!(path.display().to_string().ends_with("package.sp"));
            }
            InstallRequest::Remote(_) => panic!("Expected local file request"),
        }
    }
}

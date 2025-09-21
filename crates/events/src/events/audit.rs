use serde::{Deserialize, Serialize};

/// Audit and vulnerability scanning events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuditEvent {
    /// Audit scan started
    ScanStarted { package_count: usize },

    /// Audit of a single package completed
    ScanPackageCompleted {
        package: String,
        vulnerabilities_found: usize,
    },

    /// Audit scan completed
    ScanCompleted {
        packages_scanned: usize,
        vulnerabilities_found: usize,
        critical_count: usize,
    },

    /// Audit scan failed
    ScanFailed { retryable: bool },

    /// Vulnerability database update started
    VulnDbUpdateStarted,

    /// Vulnerability database update completed
    VulnDbUpdateCompleted {
        total_vulnerabilities: usize,
        sources_updated: usize,
        duration_ms: u64,
    },

    /// Vulnerability database update failed
    VulnDbUpdateFailed { retryable: bool },
}

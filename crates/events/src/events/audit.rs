use serde::{Deserialize, Serialize};

/// Audit and vulnerability scanning events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuditEvent {
    /// Audit scan starting
    Starting { package_count: usize },

    /// Audit package completed
    PackageCompleted {
        package: String,
        vulnerabilities_found: usize,
    },

    /// Audit scan completed
    Completed {
        packages_scanned: usize,
        vulnerabilities_found: usize,
        critical_count: usize,
    },

    /// Vulnerability database update starting
    VulnDbUpdateStarting,

    /// Vulnerability database source update starting
    VulnDbSourceUpdateStarting { source: String },

    /// Vulnerability database source update completed
    VulnDbSourceUpdateCompleted {
        source: String,
        vulnerabilities_added: usize,
        duration_ms: u64,
    },

    /// Vulnerability database source update failed
    VulnDbSourceUpdateFailed { source: String, error: String },

    /// Vulnerability database update completed
    VulnDbUpdateCompleted {
        total_vulnerabilities: usize,
        sources_updated: usize,
        duration_ms: u64,
    },
}

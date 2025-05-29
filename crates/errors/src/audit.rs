//! Audit system error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AuditError {
    #[error("SBOM parsing failed: {message}")]
    SbomParseError { message: String },

    #[error("vulnerability database error: {message}")]
    DatabaseError { message: String },

    #[error("CVE data fetch failed: {message}")]
    CveFetchError { message: String },

    #[error("invalid CVE ID: {id}")]
    InvalidCveId { id: String },

    #[error("SBOM file not found: {path}")]
    SbomNotFound { path: String },

    #[error("vulnerability scan failed: {message}")]
    ScanFailed { message: String },

    #[error("database connection failed: {message}")]
    ConnectionFailed { message: String },

    #[error("invalid vulnerability data: {message}")]
    InvalidData { message: String },

    #[error("audit operation timeout after {seconds} seconds")]
    Timeout { seconds: u64 },

    #[error("unsupported SBOM format: {format}")]
    UnsupportedFormat { format: String },

    #[error("critical vulnerabilities found: {count}")]
    CriticalVulnerabilitiesFound { count: usize },

    #[error("not implemented: {feature}")]
    NotImplemented { feature: String },
}
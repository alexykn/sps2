//! Audit system error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
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

    #[error("scan error: {message}")]
    ScanError { message: String },

    #[error("scan timeout for component {component} after {timeout_seconds} seconds")]
    ScanTimeout {
        component: String,
        timeout_seconds: u64,
    },
}

impl UserFacingError for AuditError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::CveFetchError { .. } | Self::ConnectionFailed { .. } => {
                Some("Check network connectivity to the vulnerability database and retry.")
            }
            Self::SbomNotFound { .. } => Some("Provide a valid SBOM file path or generate one before auditing."),
            Self::Timeout { .. } | Self::ScanTimeout { .. } => Some("Increase the audit timeout or retry when the system is idle."),
            Self::CriticalVulnerabilitiesFound { .. } => {
                Some("Address the critical vulnerabilities or acknowledge them with the appropriate flag.")
            }
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::CveFetchError { .. }
                | Self::ConnectionFailed { .. }
                | Self::Timeout { .. }
                | Self::ScanTimeout { .. }
        )
    }
}

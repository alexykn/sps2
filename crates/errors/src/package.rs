//! Package-related error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PackageError {
    #[error("package not found: {name}")]
    NotFound { name: String },

    #[error("package corrupted: {message}")]
    Corrupted { message: String },

    #[error("missing dependency: {name} {spec}")]
    MissingDependency { name: String, spec: String },

    #[error("dependency conflict: {message}")]
    DependencyConflict { message: String },

    #[error("circular dependency: {packages}")]
    CircularDependency { packages: String },

    #[error("invalid manifest: {message}")]
    InvalidManifest { message: String },

    #[error("signature verification failed: {message}")]
    SignatureVerificationFailed { message: String },

    #[error("unsigned package")]
    UnsignedPackage,

    #[error("invalid package format: {message}")]
    InvalidFormat { message: String },

    #[error("SBOM missing or invalid: {message}")]
    SbomError { message: String },

    #[error("already installed: {name} {version}")]
    AlreadyInstalled { name: String, version: String },

    #[error("dependency cycle detected: {package}")]
    DependencyCycle { package: String },

    #[error("incompatible package format version {version}: {reason}")]
    IncompatibleFormat { version: String, reason: String },

    #[error("resolution timeout: {message}")]
    ResolutionTimeout { message: String },

    #[error("source not available: {package}")]
    SourceNotAvailable { package: String },
}

impl UserFacingError for PackageError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::NotFound { .. } => Some("Run `sps2 reposync` or check the package name."),
            Self::MissingDependency { .. } => {
                Some("Add the missing dependency to your install request or build recipe.")
            }
            Self::DependencyConflict { .. } | Self::DependencyCycle { .. } => {
                Some("Adjust your requested package versions to resolve the dependency conflict.")
            }
            Self::SignatureVerificationFailed { .. } | Self::UnsignedPackage => {
                Some("Verify the package signature or supply trusted keys before proceeding.")
            }
            Self::AlreadyInstalled { .. } => {
                Some("Use `sps2 update` or `sps2 upgrade` if you want a newer version.")
            }
            Self::ResolutionTimeout { .. } => {
                Some("Retry the operation with fewer packages or increase the resolver timeout.")
            }
            Self::SourceNotAvailable { .. } => {
                Some("Ensure the source repository is reachable or configured.")
            }
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ResolutionTimeout { .. } | Self::SourceNotAvailable { .. }
        )
    }
}

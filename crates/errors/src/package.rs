//! Package-related error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
}

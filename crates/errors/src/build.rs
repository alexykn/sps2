//! Build system error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BuildError {
    #[error("build failed: {message}")]
    Failed { message: String },

    #[error("recipe error: {message}")]
    RecipeError { message: String },

    #[error("missing build dependency: {name}")]
    MissingBuildDep { name: String },

    #[error("fetch failed: {url}")]
    FetchFailed { url: String },

    #[error("patch failed: {patch}")]
    PatchFailed { patch: String },

    #[error("configure failed: {message}")]
    ConfigureFailed { message: String },

    #[error("compile failed: {message}")]
    CompileFailed { message: String },

    #[error("install failed: {message}")]
    InstallFailed { message: String },

    #[error("sandbox violation: {message}")]
    SandboxViolation { message: String },

    #[error("network access denied")]
    NetworkAccessDenied,

    #[error("build timeout after {seconds} seconds")]
    Timeout { seconds: u64 },

    #[error("hash mismatch for {file}: expected {expected}, got {actual}")]
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    #[error("SBOM error: {message}")]
    SbomError { message: String },

    #[error("build timeout for {package} after {timeout_seconds} seconds")]
    BuildTimeout {
        package: String,
        timeout_seconds: u64,
    },

    #[error("extraction failed: {message}")]
    ExtractionFailed { message: String },

    #[error("network access disabled for {url}")]
    NetworkDisabled { url: String },

    #[error("invalid URL: {url}")]
    InvalidUrl { url: String },

    #[error("signing error: {message}")]
    SigningError { message: String },
}

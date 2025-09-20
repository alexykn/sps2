//! Build system error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
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

    #[error("no build system detected in {path}")]
    NoBuildSystemDetected { path: String },

    #[error("dependency conflict: {message}")]
    DependencyConflict { message: String },

    #[error("compilation failed: {message}")]
    CompilationFailed { message: String },

    #[error("tests failed: {passed}/{total} tests passed")]
    TestsFailed { passed: usize, total: usize },

    #[error("quality assurance failed: {message}")]
    QualityAssuranceFailed { message: String },

    #[error("linter error: {linter} - {message}")]
    LinterError { linter: String, message: String },

    #[error("security vulnerability found: {scanner} - {message}")]
    SecurityVulnerability { scanner: String, message: String },

    #[error("policy violation: {rule} - {message}")]
    PolicyViolation { rule: String, message: String },

    #[error("license compliance error: {message}")]
    LicenseComplianceError { message: String },

    #[error("draft metadata extraction failed: {message}")]
    DraftMetadataFailed { message: String },

    #[error("draft template rendering failed: {message}")]
    DraftTemplateFailed { message: String },

    #[error("draft source preparation failed: {message}")]
    DraftSourceFailed { message: String },

    #[error("unsupported archive format: {format}")]
    UnsupportedArchiveFormat { format: String },

    #[error("git clone failed: {message}")]
    GitCloneFailed { message: String },

    #[error("validation failed: {message}")]
    ValidationFailed { message: String },

    #[error("dangerous command blocked: {command} - {reason}")]
    DangerousCommand { command: String, reason: String },

    #[error("invalid path: {path} - {reason}")]
    InvalidPath { path: String, reason: String },

    #[error("invalid URL: {url} - {reason}")]
    InvalidUrlValidation { url: String, reason: String },

    #[error("command parsing failed: {command} - {reason}")]
    CommandParseError { command: String, reason: String },

    #[error("path escape attempt: {path} resolves to {resolved} outside build root {build_root}")]
    PathEscapeAttempt {
        path: String,
        resolved: String,
        build_root: String,
    },

    #[error("dangerous write operation to {path}")]
    DangerousWrite { path: String },

    #[error("dangerous execution of {path}")]
    DangerousExecution { path: String },

    #[error("symlink loop detected at {path}")]
    SymlinkLoop { path: String },

    #[error("too many symlinks while resolving {path}")]
    TooManySymlinks { path: String },

    #[error("path traversal attempt: {path} - {reason}")]
    PathTraversalAttempt { path: String, reason: String },

    #[error("disallowed command: {command}")]
    DisallowedCommand { command: String },
}

impl UserFacingError for BuildError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::MissingBuildDep { .. } => {
                Some("Install the missing build dependency or declare it in the recipe.")
            }
            Self::FetchFailed { .. } | Self::InvalidUrl { .. } | Self::NetworkDisabled { .. } => {
                Some("Check network access or provide local source artifacts for the build.")
            }
            Self::NetworkAccessDenied => {
                Some("Allow network access for the build or supply pre-fetched sources.")
            }
            Self::PatchFailed { .. } => {
                Some("Update the patch so it applies cleanly to the current sources.")
            }
            Self::Timeout { .. } | Self::BuildTimeout { .. } => {
                Some("Increase the build timeout or reduce parallelism, then retry.")
            }
            Self::SigningError { .. } => {
                Some("Verify signing configuration and ensure the required keys are available.")
            }
            Self::RecipeError { .. }
            | Self::InvalidPath { .. }
            | Self::InvalidUrlValidation { .. } => {
                Some("Correct the recipe definition before retrying the build.")
            }
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::FetchFailed { .. } | Self::Timeout { .. } | Self::BuildTimeout { .. }
        )
    }
}

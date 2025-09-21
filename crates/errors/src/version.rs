//! Version and constraint parsing error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum VersionError {
    #[error("invalid version: {input}")]
    InvalidVersion { input: String },

    #[error("invalid version constraint: {input}")]
    InvalidConstraint { input: String },

    #[error("incompatible version: {version} does not satisfy {constraint}")]
    IncompatibleVersion { version: String, constraint: String },

    #[error("no version satisfies constraints: {constraints}")]
    NoSatisfyingVersion { constraints: String },

    #[error("version parse error: {message}")]
    ParseError { message: String },
}

impl UserFacingError for VersionError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::InvalidVersion { .. } | Self::ParseError { .. } => {
                Some("Use semantic-version strings like 1.2.3 or consult the package's available versions.")
            }
            Self::InvalidConstraint { .. } => Some("Use caret (`^`), tilde (`~`), or equality constraints accepted by sps2."),
            Self::IncompatibleVersion { .. } | Self::NoSatisfyingVersion { .. } => {
                Some("Relax the version requirement or select a different package build.")
            }
        }
    }

    fn is_retryable(&self) -> bool {
        false
    }

    fn user_code(&self) -> Option<&'static str> {
        let code = match self {
            Self::InvalidVersion { .. } => "version.invalid_version",
            Self::InvalidConstraint { .. } => "version.invalid_constraint",
            Self::IncompatibleVersion { .. } => "version.incompatible_version",
            Self::NoSatisfyingVersion { .. } => "version.no_satisfying_version",
            Self::ParseError { .. } => "version.parse_error",
        };
        Some(code)
    }
}

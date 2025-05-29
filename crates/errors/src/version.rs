//! Version and constraint parsing error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

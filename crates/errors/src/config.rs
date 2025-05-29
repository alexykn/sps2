//! Configuration error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ConfigError {
    #[error("config file not found: {path}")]
    NotFound { path: String },

    #[error("invalid config: {message}")]
    Invalid { message: String },

    #[error("parse error: {message}")]
    ParseError { message: String },

    #[error("missing required field: {field}")]
    MissingField { field: String },

    #[error("invalid value for {field}: {value}")]
    InvalidValue { field: String, value: String },

    #[error("environment variable not found: {var}")]
    EnvVarNotFound { var: String },
}

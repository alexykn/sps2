//! Configuration error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
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

    #[error("failed to write config to {path}: {error}")]
    WriteError { path: String, error: String },

    #[error("failed to serialize config: {error}")]
    SerializeError { error: String },
}

impl UserFacingError for ConfigError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::NotFound { .. } => {
                Some("Provide a configuration file or run `sps2 setup` to create one.")
            }
            Self::MissingField { field } => Some(match field.as_str() {
                "store" => "Set the store path in the configuration file or via CLI flags.",
                _ => "Add the missing configuration field noted in the error message.",
            }),
            Self::InvalidValue { .. } | Self::Invalid { .. } | Self::ParseError { .. } => {
                Some("Fix the configuration value and retry the command.")
            }
            Self::EnvVarNotFound { .. } => {
                Some("Export the environment variable or move the value into the config file.")
            }
            Self::WriteError { .. } => Some("Ensure the config path is writable and retry."),
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        false
    }
}

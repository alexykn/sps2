//! CLI error handling

use std::fmt;

/// CLI-specific error type
#[derive(Debug)]
pub enum CliError {
    /// Configuration error
    Config(sps2_errors::ConfigError),
    /// Operations error
    Ops(sps2_errors::Error),
    /// System setup error
    Setup(String),
    /// Event channel closed unexpectedly
    EventChannelClosed,
    /// Invalid command arguments
    InvalidArguments(String),
    /// I/O error
    Io(std::io::Error),
    /// Recovery error
    RecoveryError(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Config(e) => write!(f, "Configuration error: {e}"),
            CliError::Ops(e) => write!(f, "{e}"),
            CliError::Setup(msg) => write!(f, "System setup error: {msg}"),
            CliError::EventChannelClosed => write!(f, "Internal communication error"),
            CliError::InvalidArguments(msg) => write!(f, "Invalid arguments: {msg}"),
            CliError::Io(e) => write!(f, "I/O error: {e}"),
            CliError::RecoveryError(msg) => write!(f, "Recovery error: {msg}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Config(e) => Some(e),
            CliError::Ops(e) => Some(e),
            CliError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<sps2_errors::ConfigError> for CliError {
    fn from(e: sps2_errors::ConfigError) -> Self {
        CliError::Config(e)
    }
}

impl From<sps2_errors::Error> for CliError {
    fn from(e: sps2_errors::Error) -> Self {
        CliError::Ops(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}

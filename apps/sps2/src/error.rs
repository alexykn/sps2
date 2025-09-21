//! CLI error handling

use std::fmt;

use sps2_errors::UserFacingError;

/// CLI-specific error type
#[derive(Debug)]
pub enum CliError {
    /// Configuration error
    Config(sps2_errors::ConfigError),
    /// Operations error
    Ops(sps2_errors::Error),
    /// System setup error
    Setup(String),

    /// Invalid command arguments
    InvalidArguments(String),
    /// I/O error
    Io(std::io::Error),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Config(e) => write!(f, "Configuration error: {e}"),
            CliError::Ops(e) => {
                let message = e.user_message();
                write!(f, "{message}")?;
                if let Some(code) = e.user_code() {
                    write!(f, "\n  Code: {code}")?;
                }
                if let Some(hint) = e.user_hint() {
                    write!(f, "\n  Hint: {hint}")?;
                }
                if e.is_retryable() {
                    write!(f, "\n  Retry: safe to retry this operation.")?;
                }
                Ok(())
            }
            CliError::Setup(msg) => write!(f, "System setup error: {msg}"),

            CliError::InvalidArguments(msg) => write!(f, "Invalid arguments: {msg}"),
            CliError::Io(e) => write!(f, "I/O error: {e}"),
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

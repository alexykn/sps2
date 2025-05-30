//! CLI error handling

use std::fmt;

/// CLI-specific error type
#[derive(Debug)]
pub enum CliError {
    /// Configuration error
    Config(spsv2_errors::ConfigError),
    /// Operations error
    Ops(spsv2_errors::Error),
    /// System setup error
    Setup(String),
    /// Event channel closed unexpectedly
    EventChannelClosed,
    /// Invalid command arguments
    #[allow(dead_code)]
    InvalidArguments(String),
    /// I/O error
    Io(std::io::Error),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Config(e) => write!(f, "Configuration error: {}", e),
            CliError::Ops(e) => write!(f, "{}", e),
            CliError::Setup(msg) => write!(f, "System setup error: {}", msg),
            CliError::EventChannelClosed => write!(f, "Internal communication error"),
            CliError::InvalidArguments(msg) => write!(f, "Invalid arguments: {}", msg),
            CliError::Io(e) => write!(f, "I/O error: {}", e),
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

impl From<spsv2_errors::ConfigError> for CliError {
    fn from(e: spsv2_errors::ConfigError) -> Self {
        CliError::Config(e)
    }
}

impl From<spsv2_errors::Error> for CliError {
    fn from(e: spsv2_errors::Error) -> Self {
        CliError::Ops(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_error_display() {
        let error = CliError::Setup("test error".to_string());
        assert_eq!(error.to_string(), "System setup error: test error");

        let error = CliError::InvalidArguments("missing package name".to_string());
        assert_eq!(error.to_string(), "Invalid arguments: missing package name");

        let error = CliError::EventChannelClosed;
        assert_eq!(error.to_string(), "Internal communication error");
    }

    #[test]
    fn test_cli_error_from_conversions() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let cli_error = CliError::from(io_error);
        assert!(matches!(cli_error, CliError::Io(_)));
    }
}

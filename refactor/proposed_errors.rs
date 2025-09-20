//! Proposed error taxonomy with durable codes and layered context.

use std::borrow::Cow;
use thiserror::Error;

/// Stable error codes grouped by domain. Codes double as telemetry dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    ResolveTimeout,
    ResolveConflict,
    FetchNetwork,
    FetchChecksumMismatch,
    InstallFilesystem,
    InstallValidation,
    InstallRollback,
    StateDatabase,
    PlatformProcess,
    Unknown,
}

/// Severity allows quick decisions (retry, abort, warn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Recoverable,
    Fatal,
}

/// Structured error envelope returned across crate boundaries.
#[derive(Debug, Error)]
#[error("{code:?}: {message}")]
pub struct Error {
    pub code: ErrorCode,
    pub severity: Severity,
    pub message: Cow<'static, str>,
    #[source]
    pub source: Option<anyhow::Error>,
    pub context: ErrorContext,
}

/// Domain-specific context captured once and reused in events / logs.
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub operation: &'static str,
    pub package: Option<String>,
    pub version: Option<String>,
    pub path: Option<std::path::PathBuf>,
    pub url: Option<String>,
    pub retryable: bool,
}

impl Error {
    pub fn new(
        code: ErrorCode,
        severity: Severity,
        message: impl Into<Cow<'static, str>>,
        context: ErrorContext,
    ) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            source: None,
            context,
        }
    }

    pub fn with_source(mut self, source: impl Into<anyhow::Error>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn hint(&self) -> Option<&'static str> {
        match self.code {
            ErrorCode::ResolveConflict => Some("Run `sps2 resolve --explain` to inspect conflicts."),
            ErrorCode::FetchChecksumMismatch => Some("Retry with `--no-cache` or verify the upstream artifact."),
            ErrorCode::InstallRollback => Some("Check `sps2 guard heal` to restore consistency."),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

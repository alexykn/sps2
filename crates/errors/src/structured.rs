use std::borrow::Cow;
use std::collections::BTreeMap;

use thiserror::Error;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Stable error codes shared across the application and surface to clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ErrorCode {
    ResolveTimeout,
    ResolveConflict,
    ResolveInternal,
    FetchNetwork,
    FetchChecksumMismatch,
    FetchPermissionDenied,
    InstallFilesystem,
    InstallValidation,
    InstallRollback,
    InstallConflict,
    BuildCompilationFailed,
    BuildSandboxViolation,
    StateDatabase,
    StateLock,
    PlatformProcess,
    PlatformFilesystem,
    GuardVerification,
    GuardHealing,
    OpsCancelled,
    Unknown,
}

impl ErrorCode {
    /// Human friendly identifier that can be shown to end users.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ResolveTimeout => "PM0001",
            Self::ResolveConflict => "PM0002",
            Self::ResolveInternal => "PM0003",
            Self::FetchNetwork => "PM0100",
            Self::FetchChecksumMismatch => "PM0101",
            Self::FetchPermissionDenied => "PM0102",
            Self::InstallFilesystem => "PM0200",
            Self::InstallValidation => "PM0201",
            Self::InstallRollback => "PM0202",
            Self::InstallConflict => "PM0203",
            Self::BuildCompilationFailed => "PM0300",
            Self::BuildSandboxViolation => "PM0301",
            Self::StateDatabase => "PM0400",
            Self::StateLock => "PM0401",
            Self::PlatformProcess => "PM0500",
            Self::PlatformFilesystem => "PM0501",
            Self::GuardVerification => "PM0600",
            Self::GuardHealing => "PM0601",
            Self::OpsCancelled => "PM0700",
            Self::Unknown => "PM9999",
        }
    }
}

/// Severity is used to drive UI messaging and retry/backoff policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ErrorSeverity {
    Info,
    Warning,
    Recoverable,
    Fatal,
}

impl ErrorSeverity {
    #[must_use]
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Recoverable)
    }
}

/// Additional structured context that can be surfaced alongside the error.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct ErrorContext {
    pub operation: Option<Cow<'static, str>>,
    pub package: Option<String>,
    pub version: Option<String>,
    pub resource: Option<String>,
    pub hints: Vec<Cow<'static, str>>,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "BTreeMap::is_empty"))]
    pub labels: BTreeMap<String, String>,
}

impl ErrorContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_operation(mut self, operation: impl Into<Cow<'static, str>>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    #[must_use]
    pub fn with_package(mut self, package: impl Into<String>) -> Self {
        self.package = Some(package.into());
        self
    }

    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    #[must_use]
    pub fn push_hint(mut self, hint: impl Into<Cow<'static, str>>) -> Self {
        self.hints.push(hint.into());
        self
    }

    #[must_use]
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }
}

/// Structured error envelope that ties error codes, severity, and context together.
#[derive(Clone, Debug, Error)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[error("{code:?}: {message}")]
pub struct StructuredError {
    pub code: ErrorCode,
    pub severity: ErrorSeverity,
    /// Localised/user-facing message (short, < 80 chars recommended).
    pub message: Cow<'static, str>,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub details: Option<String>,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "BTreeMap::is_empty"))]
    pub metadata: BTreeMap<String, String>,
    pub context: ErrorContext,
}

impl StructuredError {
    #[must_use]
    pub fn new(
        code: ErrorCode,
        severity: ErrorSeverity,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            details: None,
            metadata: BTreeMap::new(),
            context: ErrorContext::default(),
        }
    }

    #[must_use]
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_context(mut self, context: ErrorContext) -> Self {
        self.context = context;
        self
    }

    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.context.hints.first().map(std::convert::AsRef::as_ref)
    }
}

//! Core types for quality assurance system

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Type of quality check being performed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QaCheckType {
    /// Code linting
    Linter,
    /// Security vulnerability scanning
    SecurityScanner,
    /// Policy validation
    PolicyValidator,
    /// License compliance
    LicenseCheck,
    /// File permission check
    PermissionCheck,
    /// Binary size limit
    SizeLimit,
    /// Custom policy rule
    CustomPolicy,
}

impl std::fmt::Display for QaCheckType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Linter => write!(f, "Linter"),
            Self::SecurityScanner => write!(f, "Security Scanner"),
            Self::PolicyValidator => write!(f, "Policy Validator"),
            Self::LicenseCheck => write!(f, "License Check"),
            Self::PermissionCheck => write!(f, "Permission Check"),
            Self::SizeLimit => write!(f, "Size Limit"),
            Self::CustomPolicy => write!(f, "Custom Policy"),
        }
    }
}

/// Severity level of a QA finding
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum QaSeverity {
    /// Informational finding
    Info,
    /// Warning that should be addressed
    Warning,
    /// Error that must be fixed
    Error,
    /// Critical security issue
    Critical,
}

impl std::fmt::Display for QaSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "INFO"),
            Self::Warning => write!(f, "WARNING"),
            Self::Error => write!(f, "ERROR"),
            Self::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// Individual QA check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaCheck {
    /// Type of check performed
    pub check_type: QaCheckType,
    /// Name of the specific check (e.g., "clippy", "cargo-audit")
    pub check_name: String,
    /// Severity of the finding
    pub severity: QaSeverity,
    /// Human-readable message
    pub message: String,
    /// File path if applicable
    pub file_path: Option<PathBuf>,
    /// Line number if applicable
    pub line_number: Option<usize>,
    /// Column number if applicable
    pub column_number: Option<usize>,
    /// Additional context or fix suggestions
    pub context: Option<String>,
    /// Error code if applicable (e.g., "clippy::unwrap_used")
    pub code: Option<String>,
}

impl QaCheck {
    /// Create a new QA check result
    #[must_use]
    pub fn new(
        check_type: QaCheckType,
        check_name: impl Into<String>,
        severity: QaSeverity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            check_type,
            check_name: check_name.into(),
            severity,
            message: message.into(),
            file_path: None,
            line_number: None,
            column_number: None,
            context: None,
            code: None,
        }
    }

    /// Add file location information
    #[must_use]
    pub fn with_location(
        mut self,
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    ) -> Self {
        self.file_path = Some(path);
        self.line_number = line;
        self.column_number = column;
        self
    }

    /// Add error code
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Add context or fix suggestion
    #[must_use]
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }
}

/// Language or file type being checked
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    C,
    Cpp,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Shell,
    Yaml,
    Toml,
    Json,
    Markdown,
    Other,
}

impl Language {
    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => Self::Rust,
            "c" | "h" => Self::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Self::Cpp,
            "py" | "pyi" => Self::Python,
            "js" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "tsx" => Self::TypeScript,
            "go" => Self::Go,
            "sh" | "bash" | "zsh" => Self::Shell,
            "yaml" | "yml" => Self::Yaml,
            "toml" => Self::Toml,
            "json" => Self::Json,
            "md" | "markdown" => Self::Markdown,
            _ => Self::Other,
        }
    }
}

/// Linter configuration for a specific language
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinterConfig {
    /// Linter command to run
    pub command: String,
    /// Arguments to pass to the linter
    pub args: Vec<String>,
    /// File extensions this linter handles
    pub extensions: Vec<String>,
    /// Whether this linter is enabled
    pub enabled: bool,
    /// Custom environment variables
    pub env: HashMap<String, String>,
}

/// Security scanner configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    /// Scanner command to run
    pub command: String,
    /// Arguments to pass to the scanner
    pub args: Vec<String>,
    /// Whether this scanner is enabled
    pub enabled: bool,
    /// Severity threshold (ignore findings below this level)
    pub severity_threshold: QaSeverity,
    /// Custom environment variables
    pub env: HashMap<String, String>,
}

/// Policy rule configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Unique identifier for the rule
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what this rule checks
    pub description: String,
    /// Severity if violated
    pub severity: QaSeverity,
    /// Whether this rule is enabled
    pub enabled: bool,
    /// Rule-specific configuration
    pub config: HashMap<String, serde_json::Value>,
}

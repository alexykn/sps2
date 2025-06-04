//! Python linting with ruff, black, and mypy

use super::{run_linter_command, Linter};
use crate::quality_assurance::types::{LinterConfig, QaCheck, QaCheckType, QaSeverity};
use crate::BuildContext;
use sps2_errors::Error;
use std::path::Path;

/// Ruff linter for Python code
pub struct RuffLinter;

impl RuffLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for RuffLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for RuffLinter {
    fn name(&self) -> &'static str {
        "ruff"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "py" | "pyi"))
            .unwrap_or(false)
    }

    async fn lint(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut args = config.args.clone();
        args.push("--format".to_string());
        args.push("json".to_string());
        args.push(path.display().to_string());

        let output = run_linter_command(&config.command, &args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Try to parse JSON output
        if let Ok(json_output) = serde_json::from_slice::<Vec<RuffDiagnostic>>(&output.stdout) {
            for diag in json_output {
                checks.push(diag.to_qa_check());
            }
        } else {
            // Fallback to text parsing
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(check) = parse_ruff_text_line(line) {
                    checks.push(check);
                }
            }
        }

        Ok(checks)
    }
}

/// Black formatter for Python code
pub struct BlackLinter;

impl BlackLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for BlackLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for BlackLinter {
    fn name(&self) -> &'static str {
        "black"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "py" | "pyi"))
            .unwrap_or(false)
    }

    async fn lint(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut args = config.args.clone();
        args.push("--check".to_string());
        args.push("--diff".to_string());
        args.push(path.display().to_string());

        let output = run_linter_command(&config.command, &args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Black returns non-zero if formatting is needed
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Parse files that would be reformatted
            for line in stderr.lines() {
                if line.starts_with("would reformat") {
                    if let Some(file_path) = line.split_whitespace().last() {
                        checks.push(
                            QaCheck::new(
                                QaCheckType::Linter,
                                "black",
                                QaSeverity::Warning,
                                "File needs formatting",
                            )
                            .with_location(Path::new(file_path).to_path_buf(), None, None)
                            .with_context("Run 'black' to fix formatting issues"),
                        );
                    }
                }
            }
        }

        Ok(checks)
    }
}

/// Mypy type checker for Python code
pub struct MypyLinter;

impl MypyLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for MypyLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for MypyLinter {
    fn name(&self) -> &'static str {
        "mypy"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "py" | "pyi"))
            .unwrap_or(false)
    }

    async fn lint(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut args = config.args.clone();
        args.push(path.display().to_string());

        let output = run_linter_command(&config.command, &args, path, &config.env).await?;

        let mut checks = Vec::new();
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse mypy output
        for line in stdout.lines() {
            if let Some(check) = parse_mypy_line(line) {
                checks.push(check);
            }
        }

        Ok(checks)
    }
}

/// Ruff diagnostic from JSON output
#[derive(serde::Deserialize)]
struct RuffDiagnostic {
    code: String,
    message: String,
    location: RuffLocation,
}

#[derive(serde::Deserialize)]
struct RuffLocation {
    row: usize,
    column: usize,
    file: String,
}

impl RuffDiagnostic {
    fn to_qa_check(&self) -> QaCheck {
        // Map ruff codes to severity
        let severity = match self.code.chars().next() {
            Some('E') => QaSeverity::Error,   // Error
            Some('W') => QaSeverity::Warning, // Warning
            Some('F') => QaSeverity::Error,   // Pyflakes
            Some('C') => QaSeverity::Warning, // Convention
            Some('B') => QaSeverity::Warning, // Bugbear
            _ => QaSeverity::Info,
        };

        QaCheck::new(QaCheckType::Linter, "ruff", severity, &self.message)
            .with_location(
                Path::new(&self.location.file).to_path_buf(),
                Some(self.location.row),
                Some(self.location.column),
            )
            .with_code(&self.code)
    }
}

/// Parse ruff text output line
fn parse_ruff_text_line(line: &str) -> Option<QaCheck> {
    // Example: file.py:10:5: E501 Line too long (89 > 88 characters)
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 4 {
        return None;
    }

    let file_path = Path::new(parts[0]).to_path_buf();
    let line_num = parts[1].parse::<usize>().ok();
    let col_num = parts[2].parse::<usize>().ok();

    let remaining = parts[3].trim();
    let (code, message) = if let Some(space_pos) = remaining.find(' ') {
        let code = &remaining[..space_pos];
        let message = &remaining[space_pos + 1..];
        (Some(code), message)
    } else {
        (None, remaining)
    };

    let severity = match code.and_then(|c| c.chars().next()) {
        Some('E') => QaSeverity::Error,
        Some('W') => QaSeverity::Warning,
        Some('F') => QaSeverity::Error,
        _ => QaSeverity::Warning,
    };

    let mut check = QaCheck::new(QaCheckType::Linter, "ruff", severity, message)
        .with_location(file_path, line_num, col_num);

    if let Some(code) = code {
        check = check.with_code(code);
    }

    Some(check)
}

/// Parse mypy output line
fn parse_mypy_line(line: &str) -> Option<QaCheck> {
    // Example: file.py:10: error: Incompatible return value type (got "int", expected "str")
    let parts: Vec<&str> = line.splitn(3, ':').collect();
    if parts.len() < 3 {
        return None;
    }

    let file_path = Path::new(parts[0]).to_path_buf();
    let line_num = parts[1].parse::<usize>().ok();

    let remaining = parts[2].trim();
    let (severity, message) = if remaining.starts_with("error:") {
        (
            QaSeverity::Error,
            remaining.trim_start_matches("error:").trim(),
        )
    } else if remaining.starts_with("warning:") {
        (
            QaSeverity::Warning,
            remaining.trim_start_matches("warning:").trim(),
        )
    } else if remaining.starts_with("note:") {
        (
            QaSeverity::Info,
            remaining.trim_start_matches("note:").trim(),
        )
    } else {
        return None;
    };

    Some(
        QaCheck::new(QaCheckType::Linter, "mypy", severity, message)
            .with_location(file_path, line_num, None),
    )
}

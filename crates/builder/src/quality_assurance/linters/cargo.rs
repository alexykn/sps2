//! Rust linting with Cargo (clippy and rustfmt)

use super::{parse_location, run_linter_command, Linter};
use crate::quality_assurance::types::{LinterConfig, QaCheck, QaCheckType, QaSeverity};
use crate::BuildContext;
use sps2_errors::Error;
use std::path::Path;

/// Cargo Clippy linter for Rust code
pub struct CargoLinter;

impl CargoLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CargoLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for CargoLinter {
    fn name(&self) -> &'static str {
        "clippy"
    }

    fn can_handle(&self, path: &Path) -> bool {
        // Check if there's a Cargo.toml in the path or any parent
        let mut current = path;
        loop {
            if current.join("Cargo.toml").exists() {
                return true;
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => return false,
            }
        }
    }

    async fn lint(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let output = run_linter_command(&config.command, &config.args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Parse clippy output (JSON format would be better but text works for now)
        let stderr = String::from_utf8_lossy(&output.stderr);

        for line in stderr.lines() {
            if let Some(check) = parse_clippy_line(line) {
                checks.push(check);
            }
        }

        Ok(checks)
    }
}

/// Rustfmt linter for Rust code formatting
pub struct RustfmtLinter;

impl RustfmtLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for RustfmtLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for RustfmtLinter {
    fn name(&self) -> &'static str {
        "rustfmt"
    }

    fn can_handle(&self, path: &Path) -> bool {
        // Same as clippy - check for Cargo.toml
        let mut current = path;
        loop {
            if current.join("Cargo.toml").exists() {
                return true;
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => return false,
            }
        }
    }

    async fn lint(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let output = run_linter_command(&config.command, &config.args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Rustfmt returns non-zero exit code if formatting is needed
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);

            // Parse diff output to find files that need formatting
            for line in stdout.lines() {
                if line.starts_with("Diff in") {
                    if let Some(file_path) = parse_rustfmt_diff_line(line) {
                        checks.push(
                            QaCheck::new(
                                QaCheckType::Linter,
                                "rustfmt",
                                QaSeverity::Warning,
                                "File needs formatting",
                            )
                            .with_location(file_path, None, None)
                            .with_context("Run 'cargo fmt' to fix formatting issues"),
                        );
                    }
                }
            }

            // If no specific files found but command failed, add general check
            if checks.is_empty() {
                checks.push(
                    QaCheck::new(
                        QaCheckType::Linter,
                        "rustfmt",
                        QaSeverity::Warning,
                        "Code formatting issues detected",
                    )
                    .with_context("Run 'cargo fmt' to fix formatting issues"),
                );
            }
        }

        Ok(checks)
    }
}

/// Parse a clippy output line into a QA check
fn parse_clippy_line(line: &str) -> Option<QaCheck> {
    // Example clippy output:
    // warning: unused variable: `foo`
    //  --> src/main.rs:10:5
    //  |
    // 10 |     let foo = 42;
    //  |         ^^^ help: if this is intentional, prefix it with an underscore: `_foo`
    //  |
    //  = note: `#[warn(unused_variables)]` on by default

    // Simple parser - in production would use JSON output
    if line.contains("warning:") || line.contains("error:") {
        let severity = if line.contains("error:") {
            QaSeverity::Error
        } else {
            QaSeverity::Warning
        };

        let message = line
            .split_once(':')
            .map(|(_, msg)| msg.trim())
            .unwrap_or(line);

        return Some(QaCheck::new(
            QaCheckType::Linter,
            "clippy",
            severity,
            message,
        ));
    }

    // Parse location line
    if line.trim_start().starts_with("-->") {
        if let Some(location) = line.split("-->").nth(1) {
            let parts: Vec<&str> = location.trim().split(':').collect();
            if parts.len() >= 2 {
                let _file_path = Path::new(parts[0]).to_path_buf();
                let (_line_num, _col_num) = parse_location(&parts[1..].join(":"));

                // This would need to be associated with the previous warning/error
                // For now, we'll skip it
                return None;
            }
        }
    }

    None
}

/// Parse rustfmt diff output line
fn parse_rustfmt_diff_line(line: &str) -> Option<std::path::PathBuf> {
    // Example: "Diff in /path/to/file.rs at line 1:"
    if let Some(start) = line.find("Diff in ") {
        let path_start = start + "Diff in ".len();
        if let Some(end) = line[path_start..].find(" at line") {
            let path_str = &line[path_start..path_start + end];
            return Some(Path::new(path_str).to_path_buf());
        }
    }
    None
}

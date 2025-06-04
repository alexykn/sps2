//! JavaScript/TypeScript linting with ESLint and Prettier

use super::{run_linter_command, Linter};
use crate::quality_assurance::types::{LinterConfig, QaCheck, QaCheckType, QaSeverity};
use crate::BuildContext;
use sps2_errors::Error;
use std::path::Path;

/// ESLint linter for JavaScript/TypeScript
pub struct EslintLinter;

impl EslintLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for EslintLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for EslintLinter {
    fn name(&self) -> &'static str {
        "eslint"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs"))
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
        if let Ok(json_output) = serde_json::from_slice::<Vec<EslintFileResult>>(&output.stdout) {
            for file_result in json_output {
                for message in file_result.messages {
                    checks.push(message.to_qa_check(&file_result.file_path));
                }
            }
        } else {
            // Fallback to text parsing
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(check) = parse_eslint_text_line(line) {
                    checks.push(check);
                }
            }
        }

        Ok(checks)
    }
}

/// Prettier formatter for JavaScript/TypeScript
pub struct PrettierLinter;

impl PrettierLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for PrettierLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for PrettierLinter {
    fn name(&self) -> &'static str {
        "prettier"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                matches!(
                    e,
                    "js" | "jsx"
                        | "ts"
                        | "tsx"
                        | "mjs"
                        | "cjs"
                        | "json"
                        | "css"
                        | "scss"
                        | "html"
                        | "md"
                )
            })
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
        args.push("--list-different".to_string());
        args.push(path.display().to_string());

        let output = run_linter_command(&config.command, &args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Prettier returns non-zero and lists files that need formatting
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);

            for line in stdout.lines() {
                if !line.trim().is_empty() {
                    let file_path = Path::new(line.trim()).to_path_buf();
                    checks.push(
                        QaCheck::new(
                            QaCheckType::Linter,
                            "prettier",
                            QaSeverity::Warning,
                            "File needs formatting",
                        )
                        .with_location(file_path, None, None)
                        .with_context("Run 'prettier --write' to fix formatting issues"),
                    );
                }
            }
        }

        Ok(checks)
    }
}

/// ESLint file result from JSON output
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EslintFileResult {
    file_path: String,
    messages: Vec<EslintMessage>,
}

/// ESLint message
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EslintMessage {
    rule_id: Option<String>,
    severity: u8,
    message: String,
    line: usize,
    column: usize,
}

impl EslintMessage {
    fn to_qa_check(&self, file_path: &str) -> QaCheck {
        let severity = match self.severity {
            2 => QaSeverity::Error,
            1 => QaSeverity::Warning,
            _ => QaSeverity::Info,
        };

        let mut check = QaCheck::new(QaCheckType::Linter, "eslint", severity, &self.message)
            .with_location(
                Path::new(file_path).to_path_buf(),
                Some(self.line),
                Some(self.column),
            );

        if let Some(rule_id) = &self.rule_id {
            check = check.with_code(rule_id);
        }

        check
    }
}

/// Parse ESLint text output line
fn parse_eslint_text_line(line: &str) -> Option<QaCheck> {
    // Example: /path/to/file.js:10:5: 'foo' is defined but never used. (no-unused-vars)
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 4 {
        return None;
    }

    let file_path = Path::new(parts[0]).to_path_buf();
    let line_num = parts[1].parse::<usize>().ok();
    let col_num = parts[2].parse::<usize>().ok();

    let remaining = parts[3].trim();

    // Extract rule from parentheses at the end
    let (message, rule) = if let Some(paren_pos) = remaining.rfind('(') {
        let rule_end = remaining.rfind(')')?;
        let rule = &remaining[paren_pos + 1..rule_end];
        let msg = remaining[..paren_pos].trim();
        (msg, Some(rule))
    } else {
        (remaining, None)
    };

    // Guess severity from message
    let severity = if message.contains("error") {
        QaSeverity::Error
    } else {
        QaSeverity::Warning
    };

    let mut check = QaCheck::new(QaCheckType::Linter, "eslint", severity, message)
        .with_location(file_path, line_num, col_num);

    if let Some(rule) = rule {
        check = check.with_code(rule);
    }

    Some(check)
}

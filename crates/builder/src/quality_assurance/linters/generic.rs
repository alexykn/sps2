//! Generic linter for custom file types and languages

use super::{run_linter_command, Linter};
use crate::quality_assurance::types::{LinterConfig, QaCheck, QaCheckType, QaSeverity};
use crate::BuildContext;
use sps2_errors::Error;
use std::path::Path;

/// Generic linter that can be configured for any file type
pub struct GenericLinter {
    name: String,
    extensions: Vec<String>,
}

impl GenericLinter {
    /// Create a new generic linter
    #[must_use]
    pub fn new(name: impl Into<String>, extensions: Vec<String>) -> Self {
        Self {
            name: name.into(),
            extensions,
        }
    }
}

#[async_trait::async_trait]
impl Linter for GenericLinter {
    fn name(&self) -> &str {
        &self.name
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| self.extensions.iter().any(|e| e == ext))
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

        // Parse both stdout and stderr for generic linters
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        for line in stdout.lines().chain(stderr.lines()) {
            if let Some(check) = parse_generic_line(line, &self.name) {
                checks.push(check);
            }
        }

        // If the command failed but we didn't parse any specific errors, add a general failure
        if !output.status.success() && checks.is_empty() {
            checks.push(
                QaCheck::new(
                    QaCheckType::Linter,
                    &self.name,
                    QaSeverity::Error,
                    format!(
                        "Linter failed with exit code {}",
                        output.status.code().unwrap_or(-1)
                    ),
                )
                .with_context(format!(
                    "Command: {} {}",
                    config.command,
                    args.join(" ")
                )),
            );
        }

        Ok(checks)
    }
}

/// Shell script linter (shellcheck)
pub struct ShellcheckLinter;

impl ShellcheckLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ShellcheckLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for ShellcheckLinter {
    fn name(&self) -> &'static str {
        "shellcheck"
    }

    fn can_handle(&self, path: &Path) -> bool {
        // Check extension
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "sh" | "bash" | "zsh" | "ksh") {
                return true;
            }
        }

        // Check shebang for files without extension
        if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Some(first_line) = content.lines().next() {
                    return first_line.starts_with("#!/")
                        && (first_line.contains("sh") || first_line.contains("bash"));
                }
            }
        }

        false
    }

    async fn lint(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut args = vec!["--format".to_string(), "json".to_string()];
        args.extend(config.args.clone());
        args.push(path.display().to_string());

        let output = run_linter_command("shellcheck", &args, path, &config.env).await?;

        let mut checks = Vec::new();

        // Try JSON parsing first
        if let Ok(json_output) = serde_json::from_slice::<Vec<ShellcheckDiagnostic>>(&output.stdout)
        {
            for diag in json_output {
                checks.push(diag.to_qa_check(path));
            }
        } else {
            // Fallback to generic parsing
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(check) = parse_generic_line(line, "shellcheck") {
                    checks.push(check);
                }
            }
        }

        Ok(checks)
    }
}

/// Shellcheck diagnostic
#[derive(serde::Deserialize)]
struct ShellcheckDiagnostic {
    line: usize,
    column: usize,
    level: String,
    code: u32,
    message: String,
}

impl ShellcheckDiagnostic {
    fn to_qa_check(&self, file_path: &Path) -> QaCheck {
        let severity = match self.level.as_str() {
            "error" => QaSeverity::Error,
            "warning" => QaSeverity::Warning,
            "info" | "style" => QaSeverity::Info,
            _ => QaSeverity::Warning,
        };

        QaCheck::new(QaCheckType::Linter, "shellcheck", severity, &self.message)
            .with_location(file_path.to_path_buf(), Some(self.line), Some(self.column))
            .with_code(format!("SC{}", self.code))
    }
}

/// YAML linter
pub struct YamlLinter;

impl YamlLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for YamlLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for YamlLinter {
    fn name(&self) -> &'static str {
        "yamllint"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "yaml" | "yml"))
            .unwrap_or(false)
    }

    async fn lint(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut args = config.args.clone();
        args.push("-f".to_string());
        args.push("parsable".to_string());
        args.push(path.display().to_string());

        let output = run_linter_command("yamllint", &args, path, &config.env).await?;

        let mut checks = Vec::new();
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse yamllint output
        // Format: file.yaml:1:1: [warning] missing document start "---" (document-start)
        for line in stdout.lines() {
            if let Some(check) = parse_yamllint_line(line) {
                checks.push(check);
            }
        }

        Ok(checks)
    }
}

/// Parse yamllint output line
fn parse_yamllint_line(line: &str) -> Option<QaCheck> {
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 4 {
        return None;
    }

    let file_path = Path::new(parts[0]).to_path_buf();
    let line_num = parts[1].parse::<usize>().ok();
    let col_num = parts[2].parse::<usize>().ok();

    let remaining = parts[3].trim();

    // Parse [severity] message (rule)
    let severity = if remaining.contains("[error]") {
        QaSeverity::Error
    } else if remaining.contains("[warning]") {
        QaSeverity::Warning
    } else {
        QaSeverity::Info
    };

    // Extract message and rule
    let message = remaining
        .replace("[error]", "")
        .replace("[warning]", "")
        .trim()
        .to_string();

    let (msg, rule) = if let Some(paren_pos) = message.rfind('(') {
        let rule_end = message.rfind(')')?;
        let rule = &message[paren_pos + 1..rule_end];
        let msg = message[..paren_pos].trim();
        (msg, Some(rule))
    } else {
        (message.as_str(), None)
    };

    let mut check = QaCheck::new(QaCheckType::Linter, "yamllint", severity, msg)
        .with_location(file_path, line_num, col_num);

    if let Some(rule) = rule {
        check = check.with_code(rule);
    }

    Some(check)
}

/// Parse generic linter output line
fn parse_generic_line(line: &str, linter_name: &str) -> Option<QaCheck> {
    // Skip empty lines
    if line.trim().is_empty() {
        return None;
    }

    // Common patterns to detect issues
    let severity = if line.contains("error:") || line.contains("ERROR") {
        QaSeverity::Error
    } else if line.contains("warning:") || line.contains("WARNING") || line.contains("warn:") {
        QaSeverity::Warning
    } else if line.contains("info:") || line.contains("note:") {
        QaSeverity::Info
    } else {
        // If no clear severity marker, skip unless it looks like a file:line:col pattern
        if !line.contains(':') || line.split(':').count() < 3 {
            return None;
        }
        QaSeverity::Warning
    };

    // Try to parse file:line:column: message pattern
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() >= 3 {
        // Check if first part looks like a file path
        let potential_path = parts[0];
        if potential_path.contains('/') || potential_path.contains('.') {
            let file_path = Path::new(potential_path).to_path_buf();
            let line_num = parts.get(1).and_then(|s| s.trim().parse::<usize>().ok());
            let col_num = parts.get(2).and_then(|s| s.trim().parse::<usize>().ok());
            let message = parts.get(3).map(|s| s.trim()).unwrap_or(line);

            return Some(
                QaCheck::new(QaCheckType::Linter, linter_name, severity, message)
                    .with_location(file_path, line_num, col_num),
            );
        }
    }

    // If we couldn't parse location, just return the whole line as a message
    Some(QaCheck::new(
        QaCheckType::Linter,
        linter_name,
        severity,
        line.trim(),
    ))
}

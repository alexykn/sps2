//! C/C++ linting with clang-tidy and cppcheck

use super::{run_linter_command, Linter};
use crate::quality_assurance::types::{LinterConfig, QaCheck, QaCheckType, QaSeverity};
use crate::BuildContext;
use sps2_errors::Error;
use std::path::Path;

/// Clang-tidy linter for C/C++ code
pub struct ClangTidyLinter;

impl ClangTidyLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClangTidyLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for ClangTidyLinter {
    fn name(&self) -> &'static str {
        "clang-tidy"
    }

    fn can_handle(&self, path: &Path) -> bool {
        // Check for C/C++ source files
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            matches!(ext, "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx")
        } else {
            false
        }
    }

    async fn lint(
        &self,
        _context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error> {
        // Find all C/C++ files in the directory
        let mut files = Vec::new();
        find_cpp_files(path, &mut files)?;

        if files.is_empty() {
            return Ok(Vec::new());
        }

        let mut checks = Vec::new();

        // Run clang-tidy on each file
        for file in files {
            let mut args = config.args.clone();
            args.push(file.display().to_string());

            let output = run_linter_command(&config.command, &args, path, &config.env).await?;
            let stdout = String::from_utf8_lossy(&output.stdout);

            // Parse clang-tidy output
            for line in stdout.lines() {
                if let Some(check) = parse_clang_tidy_line(line, &file) {
                    checks.push(check);
                }
            }
        }

        Ok(checks)
    }
}

/// Cppcheck linter for C/C++ code
pub struct CppcheckLinter;

impl CppcheckLinter {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CppcheckLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linter for CppcheckLinter {
    fn name(&self) -> &'static str {
        "cppcheck"
    }

    fn can_handle(&self, path: &Path) -> bool {
        // Check for C/C++ source files
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            matches!(ext, "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx")
        } else {
            false
        }
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
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Parse cppcheck output
        for line in stderr.lines() {
            if let Some(check) = parse_cppcheck_line(line) {
                checks.push(check);
            }
        }

        Ok(checks)
    }
}

/// Find all C/C++ files in a directory recursively
fn find_cpp_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), Error> {
    if dir.is_file() {
        if let Some(ext) = dir.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx") {
                files.push(dir.to_path_buf());
            }
        }
        return Ok(());
    }

    let entries = std::fs::read_dir(dir).map_err(|e| sps2_errors::BuildError::Failed {
        message: format!("Failed to read directory: {}", e),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| sps2_errors::BuildError::Failed {
            message: format!("Failed to read directory entry: {}", e),
        })?;

        let path = entry.path();
        if path.is_dir() {
            // Skip common directories
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "target" | "build" | "node_modules" | ".git") {
                    continue;
                }
            }
            find_cpp_files(&path, files)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx") {
                files.push(path);
            }
        }
    }

    Ok(())
}

/// Parse clang-tidy output line
fn parse_clang_tidy_line(line: &str, file_path: &Path) -> Option<QaCheck> {
    // Example clang-tidy output:
    // /path/to/file.cpp:10:5: warning: variable 'x' is not initialized [cppcoreguidelines-init-variables]

    if !line.contains(": warning:") && !line.contains(": error:") && !line.contains(": note:") {
        return None;
    }

    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 4 {
        return None;
    }

    let line_num = parts[1].parse::<usize>().ok();
    let col_num = parts[2].parse::<usize>().ok();

    let remaining = parts[3].trim();
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

    // Extract check name from brackets
    let (msg, code) = if let Some(bracket_pos) = message.rfind('[') {
        let code_end = message.rfind(']')?;
        let code = &message[bracket_pos + 1..code_end];
        let msg = message[..bracket_pos].trim();
        (msg, Some(code))
    } else {
        (message, None)
    };

    let mut check = QaCheck::new(QaCheckType::Linter, "clang-tidy", severity, msg).with_location(
        file_path.to_path_buf(),
        line_num,
        col_num,
    );

    if let Some(code) = code {
        check = check.with_code(code);
    }

    Some(check)
}

/// Parse cppcheck output line
fn parse_cppcheck_line(line: &str) -> Option<QaCheck> {
    // Example cppcheck output:
    // [file.cpp:10]: (error) Memory leak: ptr
    // [file.cpp:20]: (warning) Variable 'x' is assigned a value that is never used

    if !line.starts_with('[') {
        return None;
    }

    let bracket_end = line.find(']')?;
    let location = &line[1..bracket_end];

    let parts: Vec<&str> = location.split(':').collect();
    if parts.is_empty() {
        return None;
    }

    let file_path = Path::new(parts[0]).to_path_buf();
    let line_num = parts.get(1).and_then(|s| s.parse::<usize>().ok());

    let remaining = &line[bracket_end + 1..].trim();
    if !remaining.starts_with(':') {
        return None;
    }

    let remaining = remaining[1..].trim();

    // Parse severity in parentheses
    let severity = if remaining.starts_with("(error)") {
        QaSeverity::Error
    } else if remaining.starts_with("(warning)") {
        QaSeverity::Warning
    } else if remaining.starts_with("(style)") || remaining.starts_with("(performance)") {
        QaSeverity::Info
    } else {
        QaSeverity::Warning
    };

    // Extract message after severity
    let message = if let Some(pos) = remaining.find(')') {
        remaining[pos + 1..].trim()
    } else {
        remaining
    };

    Some(
        QaCheck::new(QaCheckType::Linter, "cppcheck", severity, message)
            .with_location(file_path, line_num, None),
    )
}

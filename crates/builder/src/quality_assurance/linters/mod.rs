//! Code linting integrations for multiple languages

pub mod cargo;
pub mod clang;
pub mod eslint;
pub mod generic;
pub mod python;

use super::types::{Language, LinterConfig, QaCheck, QaCheckType, QaSeverity};
use crate::events::send_event;
use crate::BuildContext;
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use std::collections::HashMap;
use std::path::Path;
use tokio::process::Command;

/// Linter trait for implementing language-specific linters
#[async_trait::async_trait]
pub trait Linter: Send + Sync {
    /// Name of the linter
    fn name(&self) -> &str;

    /// Check if this linter can handle the given file
    fn can_handle(&self, path: &Path) -> bool;

    /// Run the linter on the given path
    async fn lint(
        &self,
        context: &BuildContext,
        path: &Path,
        config: &LinterConfig,
    ) -> Result<Vec<QaCheck>, Error>;
}

/// Linter registry managing all available linters
pub struct LinterRegistry {
    linters: HashMap<String, Box<dyn Linter>>,
}

impl LinterRegistry {
    /// Create a new linter registry with all built-in linters
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            linters: HashMap::new(),
        };

        // Register built-in linters
        registry.register(Box::new(cargo::CargoLinter::new()));
        registry.register(Box::new(cargo::RustfmtLinter::new()));
        registry.register(Box::new(clang::ClangTidyLinter::new()));
        registry.register(Box::new(clang::CppcheckLinter::new()));
        registry.register(Box::new(python::RuffLinter::new()));
        registry.register(Box::new(python::BlackLinter::new()));
        registry.register(Box::new(python::MypyLinter::new()));
        registry.register(Box::new(eslint::EslintLinter::new()));
        registry.register(Box::new(eslint::PrettierLinter::new()));

        registry
    }

    /// Register a custom linter
    pub fn register(&mut self, linter: Box<dyn Linter>) {
        self.linters.insert(linter.name().to_string(), linter);
    }

    /// Get a linter by name
    pub fn get(&self, name: &str) -> Option<&dyn Linter> {
        self.linters.get(name).map(std::convert::AsRef::as_ref)
    }

    /// Run all applicable linters on a directory
    pub async fn lint_directory(
        &self,
        context: &BuildContext,
        dir: &Path,
        configs: &HashMap<String, LinterConfig>,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut all_checks = Vec::new();

        for (name, config) in configs {
            if !config.enabled {
                continue;
            }

            if let Some(linter) = self.get(name) {
                send_event(
                    context,
                    Event::OperationStarted {
                        operation: format!("Running {} linter", name),
                    },
                );

                match linter.lint(context, dir, config).await {
                    Ok(checks) => {
                        let check_count = checks.len();
                        all_checks.extend(checks);

                        send_event(
                            context,
                            Event::OperationCompleted {
                                operation: format!("{} found {} issues", name, check_count),
                                success: true,
                            },
                        );
                    }
                    Err(e) => {
                        send_event(
                            context,
                            Event::BuildWarning {
                                package: context.name.clone(),
                                message: format!("Linter {} failed: {}", name, e),
                            },
                        );

                        // Add a check for the linter failure itself
                        all_checks.push(QaCheck::new(
                            QaCheckType::Linter,
                            name,
                            QaSeverity::Warning,
                            format!("Linter failed to run: {}", e),
                        ));
                    }
                }
            }
        }

        Ok(all_checks)
    }
}

impl Default for LinterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Common helper to run a linter command and parse output
pub async fn run_linter_command<S: ::std::hash::BuildHasher>(
    command: &str,
    args: &[String],
    working_dir: &Path,
    env: &HashMap<String, String, S>,
) -> Result<std::process::Output, Error> {
    let mut cmd = Command::new(command);
    cmd.args(args).current_dir(working_dir).env_clear();

    // Copy over essential environment variables
    for (key, value) in std::env::vars() {
        if key.starts_with("PATH") || key.starts_with("HOME") || key == "USER" {
            cmd.env(&key, &value);
        }
    }

    // Add custom environment variables
    for (key, value) in env {
        cmd.env(key, value);
    }

    cmd.output().await.map_err(|e| {
        BuildError::Failed {
            message: format!("Failed to run {}: {}", command, e),
        }
        .into()
    })
}

/// Parse line:column format commonly used by linters
pub fn parse_location(location: &str) -> (Option<usize>, Option<usize>) {
    let parts: Vec<&str> = location.split(':').collect();

    let line = parts.first().and_then(|s| s.parse::<usize>().ok());
    let column = parts.get(1).and_then(|s| s.parse::<usize>().ok());

    (line, column)
}

/// Detect language from file path
pub fn detect_language(path: &Path) -> Language {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(Language::from_extension)
        .unwrap_or(Language::Other)
}

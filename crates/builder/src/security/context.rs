//! Build security context that tracks execution state

use sps2_errors::{BuildError, Error};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// Build security context that tracks execution state
#[derive(Debug, Clone)]
pub struct SecurityContext {
    /// Current working directory (absolute path)
    current_dir: PathBuf,

    /// Stack of directories for pushd/popd
    dir_stack: Vec<PathBuf>,

    /// Build root directory (e.g., /opt/pm/build/package-1.0)
    build_root: PathBuf,

    /// Environment variables including build variables
    environment: HashMap<String, String>,

    /// Command execution history for detecting patterns
    command_history: Vec<String>,

    /// Path resolution cache to detect symlink attacks
    resolved_paths: HashMap<PathBuf, PathBuf>,
}

impl SecurityContext {
    /// Create a new security context for a build
    pub fn new(build_root: PathBuf, initial_vars: HashMap<String, String>) -> Self {
        let mut environment = HashMap::new();

        // Standard build variables
        environment.insert("BUILD_ROOT".to_string(), build_root.display().to_string());
        environment.insert("BUILD_DIR".to_string(), build_root.display().to_string());
        environment.insert(
            "DESTDIR".to_string(),
            build_root.join("destdir").display().to_string(),
        );
        environment.insert("PREFIX".to_string(), "/opt/pm/live".to_string());

        // Merge with provided variables
        environment.extend(initial_vars);

        Self {
            current_dir: build_root.clone(),
            dir_stack: Vec::new(),
            build_root,
            environment,
            command_history: Vec::new(),
            resolved_paths: HashMap::new(),
        }
    }

    /// Execute a command with security validation
    pub fn execute_command(&mut self, command: &str) -> Result<ValidatedExecution, Error> {
        // Record in history
        self.command_history.push(command.to_string());

        // Parse and validate
        let parsed = super::parse_command_with_context(command, self)?;

        // Update context based on command effects
        self.apply_command_effects(&parsed);

        Ok(parsed)
    }

    /// Validate a path access in the current context
    pub fn validate_path_access(
        &self,
        path: &str,
        access_type: PathAccessType,
    ) -> Result<PathBuf, Error> {
        // Expand variables first
        let expanded = self.expand_variables(path);

        // Resolve to absolute path
        let absolute = self.resolve_path(&expanded)?;

        // Additional checks based on access type
        match access_type {
            PathAccessType::Read => {
                // Check if within build root first
                if self.is_within_build_root(&absolute)? {
                    return Ok(absolute);
                }
                // Some system paths are OK to read even outside build root
                if Self::is_safe_system_read(&absolute) {
                    return Ok(absolute);
                }
                // Otherwise, it's a path escape attempt
                Err(BuildError::PathEscapeAttempt {
                    path: path.to_string(),
                    resolved: absolute.display().to_string(),
                    build_root: self.build_root.display().to_string(),
                }
                .into())
            }
            PathAccessType::Write => {
                // Only allow writes within build root
                if self.is_within_build_root(&absolute)? {
                    return Ok(absolute);
                }
                Err(BuildError::DangerousWrite {
                    path: absolute.display().to_string(),
                }
                .into())
            }
            PathAccessType::Execute => {
                // Check if within build root first
                if self.is_within_build_root(&absolute)? {
                    return Ok(absolute);
                }
                // Allow execution of safe system tools
                if self.is_safe_executable(&absolute) {
                    return Ok(absolute);
                }
                Err(BuildError::DangerousExecution {
                    path: absolute.display().to_string(),
                }
                .into())
            }
        }
    }

    /// Expand all variables in a string
    pub fn expand_variables(&self, input: &str) -> String {
        // Handle ${VAR} style - manually parse to avoid regex dependency
        let mut expanded = String::new();
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek() == Some(&'{') {
                chars.next(); // consume '{'
                let mut var_name = String::new();
                let mut found_closing = false;

                for ch in chars.by_ref() {
                    if ch == '}' {
                        found_closing = true;
                        break;
                    }
                    var_name.push(ch);
                }

                if found_closing {
                    if let Some(value) = self.environment.get(&var_name) {
                        expanded.push_str(value);
                    } else {
                        // Keep original if variable not found
                        write!(expanded, "${{{var_name}}}").unwrap();
                    }
                } else {
                    // Malformed variable
                    write!(expanded, "${{{var_name}").unwrap();
                }
            } else if ch == '$' && chars.peek().is_some_and(|c| c.is_alphabetic() || *c == '_') {
                // Handle $VAR style
                let mut var_name = String::new();
                while let Some(&next_ch) = chars.peek() {
                    if next_ch.is_alphanumeric() || next_ch == '_' {
                        var_name.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }

                if let Some(value) = self.environment.get(&var_name) {
                    expanded.push_str(value);
                } else {
                    // Keep original if variable not found
                    expanded.push('$');
                    expanded.push_str(&var_name);
                }
            } else {
                expanded.push(ch);
            }
        }

        expanded
    }

    /// Resolve a path to absolute form in current context
    pub fn resolve_path(&self, path: &str) -> Result<PathBuf, Error> {
        let path = Path::new(path);

        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.current_dir.join(path)
        };

        // Normalize the path (resolve .., ., etc)
        super::path_resolver::normalize_path(&absolute, &self.resolved_paths, &self.build_root)
    }

    /// Apply command side effects to context
    fn apply_command_effects(&mut self, exec: &ValidatedExecution) {
        match &exec.effect {
            CommandEffect::ChangeDirectory(new_dir) => {
                self.current_dir.clone_from(new_dir);
            }
            CommandEffect::PushDirectory(new_dir) => {
                self.dir_stack.push(self.current_dir.clone());
                self.current_dir.clone_from(new_dir);
            }
            CommandEffect::PopDirectory => {
                if let Some(prev_dir) = self.dir_stack.pop() {
                    self.current_dir = prev_dir;
                }
            }
            CommandEffect::SetVariable(name, value) => {
                self.environment.insert(name.clone(), value.clone());
            }
            CommandEffect::UnsetVariable(name) => {
                self.environment.remove(name);
            }
            CommandEffect::None => {}
        }
        // No error can occur here
    }

    /// Check if a path is within the build root
    pub(crate) fn is_within_build_root(&self, path: &Path) -> Result<bool, Error> {
        // Normalize both paths for comparison
        let normalized_path =
            super::path_resolver::normalize_path(path, &self.resolved_paths, &self.build_root)?;

        // Check if path starts with build root
        Ok(normalized_path.starts_with(&self.build_root))
    }

    /// Check if a system path is safe to read
    fn is_safe_system_read(path: &Path) -> bool {
        // Allow reading from these system locations
        const SAFE_READ_PREFIXES: &[&str] = &[
            "/usr/include",
            "/usr/lib",
            "/usr/local/include",
            "/usr/local/lib",
            "/opt/pm/live", // When accessing installed packages
        ];

        SAFE_READ_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
    }

    /// Check if a path is safe to execute
    fn is_safe_executable(&self, path: &Path) -> bool {
        const SAFE_EXEC_PATHS: &[&str] =
            &["/usr/bin", "/usr/local/bin", "/bin", "/opt/pm/live/bin"];

        // Allow execution of:
        // 1. Anything within build root
        if path.starts_with(&self.build_root) {
            return true;
        }

        // 2. Standard system tools
        SAFE_EXEC_PATHS
            .iter()
            .any(|prefix| path.starts_with(prefix))
    }

    /// Get current directory
    #[allow(dead_code)]
    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    /// Get build root
    #[allow(dead_code)]
    pub fn build_root(&self) -> &Path {
        &self.build_root
    }
}

/// Types of path access to validate
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathAccessType {
    Read,
    Write,
    Execute,
}

/// Result of validated command execution
#[derive(Debug, Clone)]
pub struct ValidatedExecution {
    /// Original command
    #[allow(dead_code)]
    pub original: String,

    /// Expanded command with variables resolved
    #[allow(dead_code)]
    pub expanded: String,

    /// Parsed command structure
    pub parsed: ParsedCommand,

    /// Side effects on context
    pub effect: CommandEffect,

    /// Validated paths accessed by this command
    #[allow(dead_code)]
    pub accessed_paths: Vec<(PathBuf, PathAccessType)>,
}

/// Parsed command structure
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    /// Tokenized command
    pub tokens: Vec<crate::validation::parser::Token>,
}

/// Side effects a command has on the context
#[derive(Debug, Clone)]
pub enum CommandEffect {
    None,
    ChangeDirectory(PathBuf),
    PushDirectory(PathBuf),
    PopDirectory,
    SetVariable(String, String),
    UnsetVariable(String),
}

//! Process execution operations for macOS platform

use async_trait::async_trait;
use sps2_errors::Error;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitStatus;

use crate::core::PlatformContext;

/// Platform-specific command builder and execution
pub struct PlatformCommand {
    program: String,
    args: Vec<String>,
    current_dir: Option<PathBuf>,
    env_vars: HashMap<String, String>,
}

impl PlatformCommand {
    /// Create a new platform command
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_string(),
            args: Vec::new(),
            current_dir: None,
            env_vars: HashMap::new(),
        }
    }

    /// Add an argument to the command
    pub fn arg<S: AsRef<str>>(&mut self, arg: S) -> &mut Self {
        self.args.push(arg.as_ref().to_string());
        self
    }

    /// Add multiple arguments to the command
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for arg in args {
            self.args.push(arg.as_ref().to_string());
        }
        self
    }

    /// Set the working directory for the command
    pub fn current_dir<P: Into<PathBuf>>(&mut self, dir: P) -> &mut Self {
        self.current_dir = Some(dir.into());
        self
    }

    /// Set an environment variable for the command
    pub fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.env_vars
            .insert(key.as_ref().to_string(), value.as_ref().to_string());
        self
    }

    /// Set multiple environment variables for the command
    pub fn envs<I, K, V>(&mut self, envs: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (key, value) in envs {
            self.env_vars
                .insert(key.as_ref().to_string(), value.as_ref().to_string());
        }
        self
    }

    /// Get the program name
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Get the arguments
    pub fn get_args(&self) -> &[String] {
        &self.args
    }

    /// Get the current directory
    pub fn get_current_dir(&self) -> Option<&PathBuf> {
        self.current_dir.as_ref()
    }

    /// Get the environment variables
    pub fn get_env_vars(&self) -> &HashMap<String, String> {
        &self.env_vars
    }
}

/// Output from command execution
pub struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Trait for process execution operations
#[async_trait]
pub trait ProcessOperations: Send + Sync {
    /// Execute a command and return the output
    async fn execute_command(
        &self,
        ctx: &PlatformContext,
        cmd: PlatformCommand,
    ) -> Result<CommandOutput, Error>;

    /// Create a new command builder
    fn create_command(&self, program: &str) -> PlatformCommand;

    /// Find the path to an executable
    async fn which(&self, program: &str) -> Result<PathBuf, Error>;
}

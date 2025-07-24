//! Process execution operations for macOS platform

use async_trait::async_trait;
use std::path::PathBuf;
use std::process::ExitStatus;
use sps2_errors::Error;

use crate::core::PlatformContext;

/// Platform-specific command builder and execution
pub struct PlatformCommand {
    program: String,
    args: Vec<String>,
    current_dir: Option<PathBuf>,
}

impl PlatformCommand {
    /// Create a new platform command
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_string(),
            args: Vec::new(),
            current_dir: None,
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
    async fn execute_command(&self, ctx: &PlatformContext, cmd: PlatformCommand) -> Result<CommandOutput, Error>;
    
    /// Create a new command builder
    fn create_command(&self, program: &str) -> PlatformCommand;
    
    /// Find the path to an executable
    async fn which(&self, program: &str) -> Result<PathBuf, Error>;
}
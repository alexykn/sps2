//! Recipe validation layer
//!
//! This module validates YAML recipe steps before they are converted to
//! execution types, ensuring security and correctness.

pub mod command;
pub mod parser;
pub mod rules;

use crate::recipe::model::{ParsedStep, PostCommand};
use crate::stages::{BuildCommand, PostStep, SourceStep};
use sps2_errors::{BuildError, Error};

/// Validate and convert a source step
pub fn validate_source_step(step: &SourceStep, _recipe_dir: &std::path::Path) -> Result<(), Error> {
    match step {
        SourceStep::Fetch { url, .. }
        | SourceStep::FetchMd5 { url, .. }
        | SourceStep::FetchSha256 { url, .. }
        | SourceStep::FetchBlake3 { url, .. } => {
            validate_url(url)?;
        }
        SourceStep::Git { url, .. } => {
            validate_git_url(url)?;
        }
        SourceStep::Copy {
            src_path: Some(path),
        }
        | SourceStep::ApplyPatch { path } => {
            validate_path(path)?;
        }
        SourceStep::Copy { src_path: None } | SourceStep::Cleanup | SourceStep::Extract { .. } => {}
    }
    Ok(())
}

/// Validate and convert a parsed build step to an executable command
pub fn validate_build_step(
    step: &ParsedStep,
    sps2_config: Option<&sps2_config::Config>,
) -> Result<BuildCommand, Error> {
    match step {
        ParsedStep::Command { command } => {
            let validated = command::parse_and_validate_command(command, sps2_config)?;
            Ok(BuildCommand::Command {
                program: validated.program,
                args: validated.args,
            })
        }
        ParsedStep::Shell { shell } => {
            command::validate_shell_command(shell, sps2_config)?;
            Ok(BuildCommand::Command {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), shell.clone()],
            })
        }
        ParsedStep::Configure { configure } => {
            validate_arguments(configure)?;
            Ok(BuildCommand::Configure {
                args: configure.clone(),
            })
        }
        ParsedStep::Make { make } => {
            validate_arguments(make)?;
            Ok(BuildCommand::Make { args: make.clone() })
        }
        ParsedStep::Cmake { cmake } => {
            validate_arguments(cmake)?;
            Ok(BuildCommand::Cmake {
                args: cmake.clone(),
            })
        }
        ParsedStep::Meson { meson } => {
            validate_arguments(meson)?;
            Ok(BuildCommand::Meson {
                args: meson.clone(),
            })
        }
        ParsedStep::Cargo { cargo } => {
            validate_arguments(cargo)?;
            Ok(BuildCommand::Cargo {
                args: cargo.clone(),
            })
        }
        ParsedStep::Go { go } => {
            validate_arguments(go)?;
            Ok(BuildCommand::Go { args: go.clone() })
        }
        ParsedStep::Python { python } => {
            validate_arguments(python)?;
            Ok(BuildCommand::Python {
                args: python.clone(),
            })
        }
        ParsedStep::Nodejs { nodejs } => {
            validate_arguments(nodejs)?;
            Ok(BuildCommand::NodeJs {
                args: nodejs.clone(),
            })
        }
    }
}

/// Validate and convert a post command
pub fn validate_post_command(
    command: &PostCommand,
    sps2_config: Option<&sps2_config::Config>,
) -> Result<PostStep, Error> {
    match command {
        PostCommand::Simple(cmd) => {
            let validated = command::parse_and_validate_command(cmd, sps2_config)?;
            Ok(PostStep::Command {
                program: validated.program,
                args: validated.args,
            })
        }
        PostCommand::Shell { shell } => {
            command::validate_shell_command(shell, sps2_config)?;
            Ok(PostStep::Command {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), shell.clone()],
            })
        }
    }
}

/// Validate a URL
pub(crate) fn validate_url(url: &str) -> Result<(), Error> {
    // Block file:// URLs
    if url.starts_with("file://") {
        return Err(BuildError::InvalidUrlValidation {
            url: url.to_string(),
            reason: "file:// URLs are not allowed for security reasons".to_string(),
        }
        .into());
    }

    // Block suspicious URLs
    if rules::is_suspicious_url(url) {
        return Err(BuildError::InvalidUrlValidation {
            url: url.to_string(),
            reason: "URL appears suspicious (webhook, ngrok, non-standard port, etc.)".to_string(),
        }
        .into());
    }

    // Block localhost/internal IPs in production
    if url.contains("localhost") || url.contains("127.0.0.1") || url.contains("0.0.0.0") {
        return Err(BuildError::InvalidUrlValidation {
            url: url.to_string(),
            reason: "URLs pointing to localhost are not allowed".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate a git URL
fn validate_git_url(url: &str) -> Result<(), Error> {
    // Git URLs can be https://, git://, or ssh (git@github.com:)
    if url.starts_with("file://") {
        return Err(BuildError::InvalidUrlValidation {
            url: url.to_string(),
            reason: "file:// URLs are not allowed for git operations".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate a file path
pub(crate) fn validate_path(path: &str) -> Result<(), Error> {
    // Check if path is within allowed build environment
    if !rules::is_within_build_env(path) {
        return Err(BuildError::InvalidPath {
            path: path.to_string(),
            reason: "Path is outside the allowed build environment".to_string(),
        }
        .into());
    }

    // Additional check for path traversal attempts beyond what is_within_build_env allows
    if path.contains("../../..") {
        return Err(BuildError::InvalidPath {
            path: path.to_string(),
            reason: "Too many levels of path traversal".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate command arguments
fn validate_arguments(args: &[String]) -> Result<(), Error> {
    for arg in args {
        // Check for dangerous patterns in arguments
        if arg.contains("sudo") || arg.contains("doas") {
            return Err(BuildError::DangerousCommand {
                command: arg.clone(),
                reason: "Privilege escalation commands are not allowed".to_string(),
            }
            .into());
        }

        // Check for command substitution attempts
        if arg.contains("$(") || arg.contains('`') {
            return Err(BuildError::CommandParseError {
                command: arg.clone(),
                reason: "Command substitution in arguments is not allowed".to_string(),
            }
            .into());
        }
    }

    Ok(())
}

// Note: SecurityContext is used during execution for stateful validation.
// This module handles recipe-time validation using config.toml allowed commands.

//! Command parser integration with security context

use super::context::{
    CommandEffect, ParsedCommand, PathAccessType, SecurityContext, ValidatedExecution,
};
use crate::validation::parser::{tokenize_shell, Token};
use sps2_errors::{BuildError, Error};
use std::path::PathBuf;

/// Parse command and determine its effects and path accesses
pub fn parse_command_with_context(
    command: &str,
    context: &SecurityContext,
) -> Result<ValidatedExecution, Error> {
    // First expand variables
    let expanded = context.expand_variables(command);

    // Tokenize the expanded command
    let tokens = tokenize_shell(&expanded);

    // Analyze command and its effects
    let mut effect = CommandEffect::None;
    let mut accessed_paths = Vec::new();

    // Process all commands in the token stream
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Command(cmd) => {
                // Find the end of this command (next operator or end of tokens)
                let mut cmd_end = i + 1;
                while cmd_end < tokens.len() {
                    if matches!(tokens[cmd_end], Token::Operator(_)) {
                        break;
                    }
                    cmd_end += 1;
                }

                // Extract tokens for this command
                let cmd_tokens = &tokens[i..cmd_end];

                // Process this command
                let mut cmd_effect = CommandEffect::None;
                let mut cmd_paths = Vec::new();
                process_command(cmd, cmd_tokens, context, &mut cmd_effect, &mut cmd_paths)?;

                // Merge effects (last effect wins for things like directory changes)
                match cmd_effect {
                    CommandEffect::None => {}
                    _ => effect = cmd_effect,
                }

                // Accumulate all accessed paths
                accessed_paths.extend(cmd_paths);

                // Move to next command
                i = cmd_end;
            }
            Token::Operator(_) => {
                // Skip operators between commands
                i += 1;
            }
            _ => {
                // Skip other tokens when looking for commands
                i += 1;
            }
        }
    }

    Ok(ValidatedExecution {
        original: command.to_string(),
        expanded,
        parsed: ParsedCommand { tokens },
        effect,
        accessed_paths,
    })
}

/// Process a specific command and determine its effects
fn process_command(
    cmd: &str,
    tokens: &[Token],
    context: &SecurityContext,
    effect: &mut CommandEffect,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    match cmd {
        // Directory change commands
        "cd" => {
            if let Some(Token::Argument(path)) = tokens.get(1) {
                let resolved = context.validate_path_access(path, PathAccessType::Read)?;
                *effect = CommandEffect::ChangeDirectory(resolved.clone());
                accessed_paths.push((resolved, PathAccessType::Read));
            } else {
                // cd with no args goes to home - block this in build context
                return Err(BuildError::DangerousCommand {
                    command: "cd".to_string(),
                    reason: "cd without arguments not allowed in build context".to_string(),
                }
                .into());
            }
        }

        "pushd" => {
            if let Some(Token::Argument(path)) = tokens.get(1) {
                let resolved = context.validate_path_access(path, PathAccessType::Read)?;
                *effect = CommandEffect::PushDirectory(resolved.clone());
                accessed_paths.push((resolved, PathAccessType::Read));
            } else {
                return Err(BuildError::DangerousCommand {
                    command: "pushd".to_string(),
                    reason: "pushd without arguments not allowed".to_string(),
                }
                .into());
            }
        }

        "popd" => {
            *effect = CommandEffect::PopDirectory;
        }

        // Variable manipulation
        "export" => {
            if let Some(Token::Argument(assignment)) = tokens.get(1) {
                if let Some((name, value)) = assignment.split_once('=') {
                    // Check for dangerous variables
                    if is_dangerous_variable(name) {
                        return Err(BuildError::DangerousCommand {
                            command: format!("export {name}"),
                            reason: format!("Setting {name} is not allowed"),
                        }
                        .into());
                    }
                    *effect = CommandEffect::SetVariable(name.to_string(), value.to_string());
                }
            }
        }

        "unset" => {
            if let Some(Token::Argument(name)) = tokens.get(1) {
                *effect = CommandEffect::UnsetVariable(name.to_string());
            }
        }

        // File operations - validate all paths
        "cp" | "mv" => {
            process_source_dest_command(tokens, context, accessed_paths)?;
        }

        "rm" | "rmdir" => {
            process_delete_command(tokens, context, accessed_paths)?;
        }

        "mkdir" | "touch" => {
            process_create_command(tokens, context, accessed_paths)?;
        }

        "cat" | "head" | "tail" | "less" | "more" | "grep" | "sed" | "awk" => {
            process_read_command(tokens, context, accessed_paths);
        }

        "chmod" | "chown" | "chgrp" => {
            process_metadata_command(tokens, context, accessed_paths)?;
        }

        "ln" => {
            process_link_command(tokens, context, accessed_paths)?;
        }

        "find" => {
            process_find_command(tokens, context, accessed_paths)?;
        }

        // Archive operations
        "tar" | "zip" | "unzip" => {
            process_archive_command(cmd, tokens, context, accessed_paths)?;
        }

        // Build tools - generally safe but check paths
        "make" | "cmake" | "gcc" | "clang" | "cc" | "c++" => {
            process_build_tool_command(tokens, context, accessed_paths)?;
        }

        // Install command
        "install" => {
            process_install_command(tokens, context, accessed_paths)?;
        }

        // Direct execution
        _ if cmd.starts_with("./") || cmd.starts_with('/') => {
            let resolved = context.validate_path_access(cmd, PathAccessType::Execute)?;
            accessed_paths.push((resolved, PathAccessType::Execute));

            // Also check arguments for paths
            check_arguments_for_paths(tokens, context, accessed_paths);
        }

        // Other commands - scan arguments for paths
        _ => {
            check_arguments_for_paths(tokens, context, accessed_paths);
        }
    }

    Ok(())
}

/// Process commands that have source and destination paths
fn process_source_dest_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    let mut arg_count = 0;
    let total_args = tokens
        .iter()
        .skip(1)
        .filter(|t| matches!(t, Token::Argument(arg) if !arg.starts_with('-')))
        .count();

    for token in tokens.iter().skip(1) {
        if let Token::Argument(arg) = token {
            if !arg.starts_with('-') {
                arg_count += 1;
                let access_type = if arg_count == total_args {
                    PathAccessType::Write // Last arg is destination
                } else {
                    PathAccessType::Read // Others are sources
                };
                let resolved = context.validate_path_access(arg, access_type)?;
                accessed_paths.push((resolved, access_type));
            }
        }
    }
    Ok(())
}

/// Process deletion commands
fn process_delete_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    for token in tokens.iter().skip(1) {
        if let Token::Argument(arg) = token {
            if !arg.starts_with('-') {
                let resolved = context.validate_path_access(arg, PathAccessType::Write)?;
                accessed_paths.push((resolved, PathAccessType::Write));
            }
        }
    }
    Ok(())
}

/// Process creation commands
fn process_create_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    for token in tokens.iter().skip(1) {
        if let Token::Argument(arg) = token {
            if !arg.starts_with('-') {
                let resolved = context.validate_path_access(arg, PathAccessType::Write)?;
                accessed_paths.push((resolved, PathAccessType::Write));
            }
        }
    }
    Ok(())
}

/// Process read-only commands
fn process_read_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) {
    for token in tokens.iter().skip(1) {
        if let Token::Argument(arg) = token {
            if !arg.starts_with('-') && looks_like_path(arg) {
                if let Ok(resolved) = context.validate_path_access(arg, PathAccessType::Read) {
                    accessed_paths.push((resolved, PathAccessType::Read));
                }
            }
        }
    }
}

/// Process metadata modification commands
fn process_metadata_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    let mut skip_next = false;
    for token in tokens.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }

        if let Token::Argument(arg) = token {
            if arg.starts_with('-') {
                // Some options take a value
                if arg == "-R" || arg == "-h" {
                    continue;
                }
                skip_next = true;
            } else if !arg.chars().all(|c| c.is_numeric() || c == ':') {
                // Not a permission mode or user:group spec
                let resolved = context.validate_path_access(arg, PathAccessType::Write)?;
                accessed_paths.push((resolved, PathAccessType::Write));
            }
        }
    }
    Ok(())
}

/// Process ln command
fn process_link_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    let args: Vec<&str> = tokens
        .iter()
        .skip(1)
        .filter_map(|t| match t {
            Token::Argument(arg) if !arg.starts_with('-') => Some(arg.as_str()),
            _ => None,
        })
        .collect();

    if args.len() >= 2 {
        // Source (target of link) - read access
        let source = context.validate_path_access(args[0], PathAccessType::Read)?;
        accessed_paths.push((source, PathAccessType::Read));

        // Destination (link name) - write access
        let dest = context.validate_path_access(args[1], PathAccessType::Write)?;
        accessed_paths.push((dest, PathAccessType::Write));
    }

    Ok(())
}

/// Process find command
fn process_find_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    // First non-option argument is the search path
    for token in tokens.iter().skip(1) {
        if let Token::Argument(arg) = token {
            if !arg.starts_with('-') {
                let resolved = context.validate_path_access(arg, PathAccessType::Read)?;
                accessed_paths.push((resolved, PathAccessType::Read));
                break; // Only first path for find
            }
        }
    }
    Ok(())
}

/// Process archive commands
fn process_archive_command(
    cmd: &str,
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    match cmd {
        "tar" => {
            // Detect if creating or extracting
            let is_creating = tokens
                .iter()
                .any(|t| matches!(t, Token::Argument(arg) if arg.contains('c')));

            // Find archive file (usually after -f)
            let mut next_is_archive = false;
            for token in tokens.iter().skip(1) {
                if let Token::Argument(arg) = token {
                    if next_is_archive {
                        let access_type = if is_creating {
                            PathAccessType::Write
                        } else {
                            PathAccessType::Read
                        };
                        let resolved = context.validate_path_access(arg, access_type)?;
                        accessed_paths.push((resolved, access_type));
                        next_is_archive = false;
                    } else if arg.contains('f') {
                        next_is_archive = true;
                    } else if !arg.starts_with('-') {
                        // Other paths are sources/targets
                        let resolved = context.validate_path_access(arg, PathAccessType::Read)?;
                        accessed_paths.push((resolved, PathAccessType::Read));
                    }
                }
            }
        }
        _ => {
            // For other archive tools, check all path-like arguments
            check_arguments_for_paths(tokens, context, accessed_paths);
        }
    }
    Ok(())
}

/// Process build tool commands
fn process_build_tool_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    for token in tokens.iter().skip(1) {
        if let Token::Argument(arg) = token {
            if let Some(output_arg) = arg.strip_prefix("-o") {
                // Output file
                let output = if !output_arg.is_empty() {
                    output_arg // -ofile
                } else if let Some(Token::Argument(next)) = tokens.get(tokens.len()) {
                    next // -o file
                } else {
                    continue;
                };
                let resolved = context.validate_path_access(output, PathAccessType::Write)?;
                accessed_paths.push((resolved, PathAccessType::Write));
            } else if looks_like_source_file(arg) {
                if let Ok(resolved) = context.validate_path_access(arg, PathAccessType::Read) {
                    accessed_paths.push((resolved, PathAccessType::Read));
                }
            }
        }
    }
    Ok(())
}

/// Process install command
fn process_install_command(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) -> Result<(), Error> {
    let mut mode_next = false;
    let mut dir_mode = false;

    // Check for -d flag (directory creation mode)
    for token in tokens {
        if let Token::Argument(arg) = token {
            if arg == "-d" {
                dir_mode = true;
                break;
            }
        }
    }

    let paths: Vec<&str> = tokens
        .iter()
        .skip(1)
        .filter_map(|t| match t {
            Token::Argument(arg) if !arg.starts_with('-') => {
                if mode_next {
                    mode_next = false;
                    None
                } else {
                    Some(arg.as_str())
                }
            }
            Token::Argument(arg) if arg == "-m" => {
                mode_next = true;
                None
            }
            _ => None,
        })
        .collect();

    if dir_mode {
        // All arguments are directories to create
        for path in paths {
            let resolved = context.validate_path_access(path, PathAccessType::Write)?;
            accessed_paths.push((resolved, PathAccessType::Write));
        }
    } else if paths.len() >= 2 {
        // Last is destination, others are sources
        for (i, path) in paths.iter().enumerate() {
            let access_type = if i == paths.len() - 1 {
                PathAccessType::Write
            } else {
                PathAccessType::Read
            };
            let resolved = context.validate_path_access(path, access_type)?;
            accessed_paths.push((resolved, access_type));
        }
    }

    Ok(())
}

/// Check arguments for paths and validate them
fn check_arguments_for_paths(
    tokens: &[Token],
    context: &SecurityContext,
    accessed_paths: &mut Vec<(PathBuf, PathAccessType)>,
) {
    for token in tokens.iter().skip(1) {
        if let Token::Argument(arg) = token {
            if looks_like_path(arg) {
                // Conservative: assume read access for unknown commands
                if let Ok(resolved) = context.validate_path_access(arg, PathAccessType::Read) {
                    accessed_paths.push((resolved, PathAccessType::Read));
                }
            }
        }
    }
}

/// Heuristic to detect if an argument looks like a path
fn looks_like_path(arg: &str) -> bool {
    arg.starts_with('/')
        || arg.starts_with("./")
        || arg.starts_with("../")
        || arg.contains('/')
        || arg == "."
        || arg == ".."
}

/// Check if argument looks like a source file
fn looks_like_source_file(arg: &str) -> bool {
    const SOURCE_EXTENSIONS: &[&str] = &[
        ".c", ".cc", ".cpp", ".cxx", ".h", ".hpp", ".hxx", ".s", ".S", ".asm", ".o", ".a", ".so",
        ".dylib",
    ];

    SOURCE_EXTENSIONS.iter().any(|ext| arg.ends_with(ext))
}

/// Check if a variable name is dangerous to set
fn is_dangerous_variable(name: &str) -> bool {
    const DANGEROUS_VARS: &[&str] = &[
        "PATH",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "LD_PRELOAD",
        "HOME",
        "USER",
        "SHELL",
    ];

    DANGEROUS_VARS.contains(&name)
}

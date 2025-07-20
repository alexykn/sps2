//! Stage-specific execution functions

use crate::security::SecurityContext;
use crate::stages::{BuildCommand, EnvironmentStep, PostStep, SourceStep};
use crate::utils::events::send_event;
use crate::{BuildCommandResult, BuildContext, BuildEnvironment, BuilderApi};
use sps2_errors::Error;
use sps2_events::Event;
use std::path::Path;

/// Check if a file is an archive that should be extracted
fn is_archive(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(ext, "gz" | "tgz" | "bz2" | "xz" | "zip")
    } else {
        // For files without extensions (like GitHub API downloads), check the file content
        use std::fs::File;
        use std::io::Read;

        if let Ok(mut file) = File::open(path) {
            let mut magic = [0u8; 4];
            if file.read_exact(&mut magic).is_ok() {
                // Check for gzip magic number (1f 8b)
                if magic[0] == 0x1f && magic[1] == 0x8b {
                    return true;
                }
                // Check for ZIP magic number (50 4b)
                if magic[0] == 0x50 && magic[1] == 0x4b {
                    return true;
                }
                // Check for bzip2 magic number (42 5a)
                if magic[0] == 0x42 && magic[1] == 0x5a {
                    return true;
                }
            }
        }
        false
    }
}

/// Execute a source step
pub async fn execute_source_step(
    step: &SourceStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match step {
        SourceStep::Cleanup => {
            cleanup_directories(api, environment).await?;
        }
        SourceStep::Fetch { url, extract_to } => {
            let download_path = api.fetch(url).await?;
            // Extract immediately after download
            if is_archive(&download_path) {
                api.extract_single_download(&download_path, extract_to.as_deref())
                    .await?;
            }
        }
        SourceStep::FetchMd5 {
            url,
            md5,
            extract_to,
        } => {
            let download_path = api.fetch_md5(url, md5).await?;
            // Extract immediately after download and verification
            if is_archive(&download_path) {
                api.extract_single_download(&download_path, extract_to.as_deref())
                    .await?;
            }
        }
        SourceStep::FetchSha256 {
            url,
            sha256,
            extract_to,
        } => {
            let download_path = api.fetch_sha256(url, sha256).await?;
            // Extract immediately after download and verification
            if is_archive(&download_path) {
                api.extract_single_download(&download_path, extract_to.as_deref())
                    .await?;
            }
        }
        SourceStep::FetchBlake3 {
            url,
            blake3,
            extract_to,
        } => {
            let download_path = api.fetch_blake3(url, blake3).await?;
            // Extract immediately after download and verification
            if is_archive(&download_path) {
                api.extract_single_download(&download_path, extract_to.as_deref())
                    .await?;
            }
        }
        SourceStep::Extract { extract_to } => {
            api.extract_downloads_to(extract_to.as_deref()).await?;
        }
        SourceStep::Git { url, ref_ } => {
            api.git(url, ref_).await?;
        }
        SourceStep::Copy { src_path } => {
            api.copy(src_path.as_deref(), &environment.context).await?;
        }
        SourceStep::ApplyPatch { path } => {
            let patch_path = environment.build_prefix().join("src").join(path);
            api.apply_patch(&patch_path, environment).await?;
        }
    }
    Ok(())
}

/// Execute a build command
pub async fn execute_build_command(
    command: &BuildCommand,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match command {
        BuildCommand::Configure { args } => {
            api.configure(args, environment).await?;
        }
        BuildCommand::Make { args } => {
            api.make(args, environment).await?;
        }
        BuildCommand::Autotools { args } => {
            api.autotools(args, environment).await?;
        }
        BuildCommand::Cmake { args } => {
            api.cmake(args, environment).await?;
        }
        BuildCommand::Meson { args } => {
            api.meson(args, environment).await?;
        }
        BuildCommand::Cargo { args } => {
            api.cargo(args, environment).await?;
        }
        BuildCommand::Go { args } => {
            api.go(args, environment).await?;
        }
        BuildCommand::Python { args } => {
            api.python(args, environment).await?;
        }
        BuildCommand::NodeJs { args } => {
            api.nodejs(args, environment).await?;
        }
        BuildCommand::Command { program, args } => {
            execute_command(program, args, api, environment).await?;
        }
    }
    Ok(())
}

/// Execute a build command with security context
pub async fn execute_build_command_with_security(
    command: &BuildCommand,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
    security_context: &mut SecurityContext,
    sps2_config: Option<&sps2_config::Config>,
) -> Result<(), Error> {
    match command {
        BuildCommand::Command { program, args } => {
            // For shell commands, validate through security context
            if program == "sh" && args.len() >= 2 && args[0] == "-c" {
                // This is a shell command
                let shell_cmd = &args[1];

                // Validate through security context
                let execution = security_context.execute_command(shell_cmd)?;

                // Additional config-based validation
                if let Some(config) = sps2_config {
                    for token in &execution.parsed.tokens {
                        if let crate::validation::parser::Token::Command(cmd) = token {
                            if !config.is_command_allowed(cmd) {
                                return Err(sps2_errors::BuildError::DisallowedCommand {
                                    command: cmd.clone(),
                                }
                                .into());
                            }
                        }
                    }
                }

                // Execute the validated command
                execute_command(program, args, api, environment).await?;
            } else {
                // For direct commands, validate and execute
                let full_cmd = format!("{} {}", program, args.join(" "));
                security_context.execute_command(&full_cmd)?;
                execute_command(program, args, api, environment).await?;
            }
        }
        // For build system commands, pass through normally (they're already sandboxed)
        _ => execute_build_command(command, api, environment).await?,
    }
    Ok(())
}

/// Execute a post-processing step
pub async fn execute_post_step(
    step: &PostStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match step {
        PostStep::PatchRpaths { style, paths } => {
            api.patch_rpaths(*style, paths, environment).await?;
        }
        PostStep::FixPermissions { paths } => {
            api.fix_permissions(paths, environment)?;
        }
        PostStep::Command { program, args } => {
            execute_command(program, args, api, environment).await?;
        }
    }
    Ok(())
}

/// Execute a post-processing step with security context
///
/// # Errors
///
/// Returns an error if:
/// - Security validation fails for command execution
/// - Command is disallowed by `sps2_config`
/// - Post-processing operation fails (patch rpaths, fix permissions, etc.)
/// - Command execution fails
pub async fn execute_post_step_with_security(
    step: &PostStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
    security_context: &mut SecurityContext,
    sps2_config: Option<&sps2_config::Config>,
) -> Result<(), Error> {
    match step {
        PostStep::Command { program, args } => {
            // Validate command through security context
            if program == "sh" && args.len() >= 2 && args[0] == "-c" {
                let shell_cmd = &args[1];

                // Validate through security context
                let execution = security_context.execute_command(shell_cmd)?;

                // Additional config-based validation
                if let Some(config) = sps2_config {
                    for token in &execution.parsed.tokens {
                        if let crate::validation::parser::Token::Command(cmd) = token {
                            if !config.is_command_allowed(cmd) {
                                return Err(sps2_errors::BuildError::DisallowedCommand {
                                    command: cmd.clone(),
                                }
                                .into());
                            }
                        }
                    }
                }

                execute_command(program, args, api, environment).await?;
            } else {
                let full_cmd = format!("{} {}", program, args.join(" "));
                security_context.execute_command(&full_cmd)?;
                execute_command(program, args, api, environment).await?;
            }
        }
        // Other post steps don't need security validation
        _ => execute_post_step(step, api, environment).await?,
    }
    Ok(())
}

/// Execute an environment step
#[allow(dead_code)] // Public API for environment step execution
pub fn execute_environment_step(
    step: &EnvironmentStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match step {
        EnvironmentStep::SetIsolation { level } => {
            api.set_isolation(*level);
        }
        EnvironmentStep::WithDefaults => {
            environment.apply_default_compiler_flags();
        }
        EnvironmentStep::AllowNetwork { enabled } => {
            let _ = api.allow_network(*enabled);
        }
        EnvironmentStep::SetEnv { key, value } => {
            environment.set_env_var(key.clone(), value.clone())?;
        }
    }
    Ok(())
}

/// Execute a generic command
async fn execute_command(
    program: &str,
    args: &[String],
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<BuildCommandResult, Error> {
    // Special handling for make commands
    if program == "make" {
        api.make(args, environment).await
    } else {
        let arg_strs: Vec<&str> = args.iter().map(String::as_str).collect();
        environment
            .execute_command(program, &arg_strs, Some(&api.working_dir))
            .await
    }
}

/// Clean up directories
async fn cleanup_directories(
    api: &BuilderApi,
    environment: &BuildEnvironment,
) -> Result<(), Error> {
    use crate::utils::events::send_event;
    use sps2_events::Event;
    use std::collections::HashMap;
    use tokio::fs;

    let staging_dir = environment.staging_dir();
    send_event(
        &environment.context,
        Event::DebugLog {
            message: format!("Cleaned staging directory: {}", staging_dir.display()),
            context: HashMap::new(),
        },
    );
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).await?;
    }
    fs::create_dir_all(&staging_dir).await?;

    let source_dir = &api.working_dir;
    send_event(
        &environment.context,
        Event::DebugLog {
            message: format!("Cleaned source directory: {}", source_dir.display()),
            context: HashMap::new(),
        },
    );
    if source_dir.exists() {
        fs::remove_dir_all(source_dir).await?;
    }
    fs::create_dir_all(source_dir).await?;

    Ok(())
}

// Note: Use execute_build_commands_list_with_security instead for proper validation

/// Execute a list of build commands with security context
pub async fn execute_build_commands_list_with_security(
    context: &BuildContext,
    build_commands: &[BuildCommand],
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
    security_context: &mut SecurityContext,
    sps2_config: Option<&sps2_config::Config>,
) -> Result<(), Error> {
    for command in build_commands {
        send_event(
            context,
            Event::BuildStepStarted {
                step: format!("{command:?}"),
                package: context.name.clone(),
            },
        );

        execute_build_command_with_security(
            command,
            api,
            environment,
            security_context,
            sps2_config,
        )
        .await?;

        send_event(
            context,
            Event::BuildStepCompleted {
                step: format!("{command:?}"),
                package: context.name.clone(),
            },
        );
    }

    Ok(())
}

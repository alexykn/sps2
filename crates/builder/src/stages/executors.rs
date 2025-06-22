//! Stage-specific execution functions

use crate::stages::{BuildCommand, EnvironmentStep, PostStep, SourceStep};
use crate::utils::events::send_event;
use crate::{BuildCommandResult, BuildContext, BuildEnvironment, BuilderApi};
use sps2_errors::Error;
use sps2_events::Event;

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
        SourceStep::Fetch { url } => {
            api.fetch(url).await?;
        }
        SourceStep::FetchMd5 { url, md5 } => {
            api.fetch_md5(url, md5).await?;
        }
        SourceStep::FetchSha256 { url, sha256 } => {
            api.fetch_sha256(url, sha256).await?;
        }
        SourceStep::FetchBlake3 { url, blake3 } => {
            api.fetch_blake3(url, blake3).await?;
        }
        SourceStep::Extract => {
            api.extract_downloads().await?;
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

/// Execute an environment step
#[allow(dead_code)]
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

/// Execute a list of build commands
pub async fn execute_build_commands_list(
    context: &BuildContext,
    build_commands: &[BuildCommand],
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    for command in build_commands {
        send_event(
            context,
            Event::BuildStepStarted {
                step: format!("{command:?}"),
                package: context.name.clone(),
            },
        );

        execute_build_command(command, api, environment).await?;

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

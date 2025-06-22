//! YAML recipe execution and build step management

use crate::environment::IsolationLevel;
use crate::utils::events::send_event;
use crate::yaml::{BuildStep, RecipeMetadata};
use crate::{BuildConfig, BuildContext, BuildEnvironment, BuilderApi};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use sps2_types::package::PackageSpec;
use std::path::Path;
use tokio::fs;

/// Execute the YAML recipe and return dependencies, metadata, and install request status
pub async fn execute_recipe(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
) -> Result<(Vec<String>, Vec<PackageSpec>, RecipeMetadata, bool), Error> {
    // Execute YAML recipe using staged execution
    crate::utils::executor::execute_staged_build(config, context, environment).await
}

/// Execute a list of build steps
pub async fn execute_build_steps_list(
    context: &BuildContext,
    build_steps: &[BuildStep],
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    // Execute build steps
    for step in build_steps {
        send_event(
            context,
            Event::BuildStepStarted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            },
        );

        execute_build_step(step, api, environment).await?;

        send_event(
            context,
            Event::BuildStepCompleted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            },
        );
    }

    Ok(())
}

/// Execute a single build step
pub async fn execute_build_step(
    step: &BuildStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match step {
        // Fetch operations
        BuildStep::Fetch { url } => {
            api.fetch(url).await?;
        }
        BuildStep::FetchMd5 { url, md5 } => {
            api.fetch_md5(url, md5).await?;
        }
        BuildStep::FetchSha256 { url, sha256 } => {
            api.fetch_sha256(url, sha256).await?;
        }
        BuildStep::FetchBlake3 { url, blake3 } => {
            api.fetch_blake3(url, blake3).await?;
        }
        BuildStep::Extract => {
            api.extract_downloads().await?;
        }
        BuildStep::Git { url, ref_ } => {
            api.git(url, ref_).await?;
        }

        // Build system operations
        BuildStep::Configure { .. }
        | BuildStep::Make { .. }
        | BuildStep::Autotools { .. }
        | BuildStep::Cmake { .. }
        | BuildStep::Meson { .. }
        | BuildStep::Cargo { .. }
        | BuildStep::Go { .. }
        | BuildStep::Python { .. }
        | BuildStep::NodeJs { .. } => {
            execute_build_system_step(step, api, environment).await?;
        }

        // Basic operations
        BuildStep::Install => {
            api.install(environment)?;
        }
        BuildStep::ApplyPatch { path } => {
            api.apply_patch(Path::new(path), environment).await?;
        }
        BuildStep::Command { program, args } => {
            execute_command_step(program, args, api, environment).await?;
        }
        BuildStep::SetEnv { key, value } => {
            environment.set_env_var(key.clone(), value.clone())?;
        }
        BuildStep::WithDefaults => {
            environment.apply_default_compiler_flags();
        }
        BuildStep::AllowNetwork { enabled } => {
            let _result = api.allow_network(*enabled);
        }
        BuildStep::Cleanup => {
            cleanup_staging_directory(environment).await?;
        }
        BuildStep::Copy { src_path } => {
            api.copy(src_path.as_deref(), &environment.context).await?;
        }
        BuildStep::PatchRpaths { style, paths } => {
            api.patch_rpaths(*style, paths, environment).await?;
        }
        BuildStep::FixPermissions { paths } => {
            api.fix_permissions(paths, environment)?;
        }
        BuildStep::SetIsolation { level } => {
            if let Some(isolation_level) = IsolationLevel::from_u8(*level) {
                api.set_isolation(isolation_level);
                environment.set_isolation_level_from_recipe(isolation_level);
            } else {
                return Err(BuildError::RecipeError {
                    message: format!("Invalid isolation level: {level}"),
                }
                .into());
            }
        }
    }

    Ok(())
}

/// Execute build system specific steps
async fn execute_build_system_step(
    step: &BuildStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match step {
        BuildStep::Configure { args } => {
            api.configure(args, environment).await?;
        }
        BuildStep::Make { args } => {
            api.make(args, environment).await?;
        }
        BuildStep::Autotools { args } => {
            api.autotools(args, environment).await?;
        }
        BuildStep::Cmake { args } => {
            api.cmake(args, environment).await?;
        }
        BuildStep::Meson { args } => {
            api.meson(args, environment).await?;
        }
        BuildStep::Cargo { args } => {
            api.cargo(args, environment).await?;
        }
        BuildStep::Go { args } => {
            api.go(args, environment).await?;
        }
        BuildStep::Python { args } => {
            api.python(args, environment).await?;
        }
        BuildStep::NodeJs { args } => {
            api.nodejs(args, environment).await?;
        }
        _ => unreachable!("Only build system steps should be passed to this function"),
    }
    Ok(())
}

/// Execute a command step with proper DESTDIR handling
async fn execute_command_step(
    program: &str,
    args: &[String],
    api: &BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    // Process arguments to handle DESTDIR properly
    let processed_args: Vec<String> = if program == "make" {
        args.iter()
            .map(|arg| {
                if arg.starts_with("DESTDIR=") {
                    // Always use the absolute staging directory
                    format!("DESTDIR={}", environment.staging_dir().display())
                } else {
                    arg.clone()
                }
            })
            .collect()
    } else {
        args.to_vec()
    };

    let arg_refs: Vec<&str> = processed_args.iter().map(String::as_str).collect();
    environment
        .execute_command(program, &arg_refs, Some(&api.working_dir))
        .await?;
    Ok(())
}

/// Clean up the staging directory for the current package
async fn cleanup_staging_directory(environment: &BuildEnvironment) -> Result<(), Error> {
    let staging_dir = environment.staging_dir();
    let source_dir = environment.build_prefix().join("src");

    // Clean staging directory if it exists
    if staging_dir.exists() {
        // Remove all contents but keep the directory itself
        let mut entries = fs::read_dir(&staging_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).await?;
            } else {
                fs::remove_file(&path).await?;
            }
        }

        // Send event about cleanup
        send_event(
            environment.context(),
            Event::DebugLog {
                message: format!("Cleaned staging directory: {}", staging_dir.display()),
                context: std::collections::HashMap::new(),
            },
        );
    }

    // Clean source directory if it exists
    if source_dir.exists() {
        // Remove all contents but keep the directory itself
        let mut entries = fs::read_dir(&source_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).await?;
            } else {
                fs::remove_file(&path).await?;
            }
        }

        // Send event about cleanup
        send_event(
            environment.context(),
            Event::DebugLog {
                message: format!("Cleaned source directory: {}", source_dir.display()),
                context: std::collections::HashMap::new(),
            },
        );
    }

    Ok(())
}

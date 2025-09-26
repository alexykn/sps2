//! sps2 - Modern package manager for macOS ARM64
//!
//! This is the main CLI application that orchestrates all package management
//! operations through the ops crate.

mod cli;
mod display;
mod error;
mod events;
mod logging;
mod setup;

use crate::cli::{Cli, Commands, KeysCommands, VulnDbCommands};
use crate::display::OutputRenderer;
use crate::error::CliError;
use crate::events::EventHandler;
use crate::setup::SystemSetup;
use clap::Parser;
use sps2_config::{fixed_paths, Config};
use sps2_events::{EventReceiver, EventSender};
use sps2_ops::{OperationResult, OpsContextBuilder};
use sps2_state::StateManager;
use sps2_types::state::TransactionPhase;
use std::process;
use tokio::select;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    // Parse command line arguments first to check for JSON mode
    let cli = Cli::parse();
    let json_mode = cli.global.json;

    // Initialize tracing with JSON awareness
    init_tracing(json_mode, cli.global.debug);

    // Run the application and handle errors
    if let Err(e) = run(cli).await {
        error!("Application error: {}", e);
        if !json_mode {
            eprintln!("Error: {e}");
        }
        process::exit(1);
    }
}

/// Main application logic
async fn run(cli: Cli) -> Result<(), CliError> {
    info!("Starting sps2 v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration with proper precedence:
    // 1. Start with file config (or defaults)
    let mut config =
        Config::load_or_default_with_builder(&cli.global.config, &cli.global.builder_config)
            .await?;

    // 2. Merge environment variables
    config.merge_env()?;

    // 3. Apply CLI flags (highest precedence)
    apply_cli_config(&mut config, &cli.global, &cli.command)?;

    // Initialize system setup
    let mut setup = SystemSetup::new(config.clone());

    // Perform startup checks and initialization
    setup.initialize().await?;

    // --- RECOVERY LOGIC ---
    // Check for and complete any interrupted transactions
    if let Err(e) = recover_if_needed(setup.state()).await {
        error!("CRITICAL ERROR: A previous operation was interrupted and could not be automatically recovered: {}", e);
        if !cli.global.json {
            eprintln!("CRITICAL ERROR: A previous operation was interrupted and could not be automatically recovered: {e}");
            eprintln!("The package manager is in a potentially inconsistent state. Please report this issue.");
        }
        return Err(e);
    }
    // --- END RECOVERY LOGIC ---

    // Create event channel
    let (event_sender, event_receiver) = sps2_events::channel();

    // Build operations context
    let ops_ctx = build_ops_context(
        &setup,
        event_sender.clone(),
        config.clone(),
        cli.global.check,
    )
    .await?;

    // Create output renderer
    let renderer = OutputRenderer::new(
        cli.global.json,
        cli.global.color.unwrap_or(config.general.color),
    );

    // Create event handler
    let colors_enabled = match cli.global.color.unwrap_or(config.general.color) {
        sps2_types::ColorChoice::Always => true,
        sps2_types::ColorChoice::Never => false,
        sps2_types::ColorChoice::Auto => console::Term::stdout().features().colors_supported(),
    };
    let mut event_handler = EventHandler::new(colors_enabled, cli.global.debug);

    // Execute command with event handling
    let result =
        execute_command_with_events(cli.command, ops_ctx, event_receiver, &mut event_handler)
            .await?;

    // Render final result
    renderer.render_result(&result)?;

    // Show PATH reminder if this was an install operation and PATH not set
    if matches!(result, OperationResult::InstallReport(_)) {
        show_path_reminder_if_needed();
    }

    info!("Command completed successfully");
    Ok(())
}

/// Execute command with concurrent event handling
async fn execute_command_with_events(
    command: Commands,
    ops_ctx: sps2_ops::OpsCtx,
    mut event_receiver: EventReceiver,
    event_handler: &mut EventHandler,
) -> Result<OperationResult, CliError> {
    let mut command_future = Box::pin(execute_command(command, ops_ctx));

    // Handle events concurrently with command execution
    loop {
        select! {
            // Command completed
            result = &mut command_future => {
                // Drain any remaining events
                while let Ok(event) = event_receiver.try_recv() {
                    event_handler.handle_event(event);
                }
                return result;
            }

            // Event received
            event = event_receiver.recv() => {
                match event {
                    Some(event) => event_handler.handle_event(event),
                    None => { /* Channel closed: keep waiting for command to finish */ }
                }
            }
        }
    }
}

/// Execute the specified command
async fn execute_command(
    command: Commands,
    ctx: sps2_ops::OpsCtx,
) -> Result<OperationResult, CliError> {
    match command {
        // Small operations (implemented in ops crate)
        Commands::Reposync { yes } => {
            let result = sps2_ops::reposync(&ctx, yes).await?;
            Ok(OperationResult::Success(result))
        }

        Commands::Repo(repo_cmd) => match repo_cmd {
            cli::RepoCommands::Add { name, url } => {
                let result = sps2_ops::small_ops::add_repo(&ctx, &name, &url).await?;
                Ok(OperationResult::Success(result))
            }
            cli::RepoCommands::List => {
                let result = sps2_ops::small_ops::list_repos(&ctx).await?;
                Ok(OperationResult::Success(result))
            }
            cli::RepoCommands::Remove { name } => {
                let result = sps2_ops::small_ops::remove_repo(&ctx, &name).await?;
                Ok(OperationResult::Success(result))
            }
        },

        Commands::Keys(keys_cmd) => match keys_cmd {
            KeysCommands::List => {
                let result = sps2_ops::keys::keys_list(&ctx).await?;
                Ok(OperationResult::Success(result))
            }
            KeysCommands::Import { file, comment } => {
                let result = sps2_ops::keys::keys_import_from_file(&ctx, &file, comment).await?;
                Ok(OperationResult::Success(result))
            }
            KeysCommands::Remove { key_id } => {
                let result = sps2_ops::keys::keys_remove(&ctx, &key_id).await?;
                Ok(OperationResult::Success(result))
            }
        },

        Commands::List => {
            let packages = sps2_ops::list_packages(&ctx).await?;
            Ok(OperationResult::PackageList(packages))
        }

        Commands::Info { package } => {
            let info = sps2_ops::package_info(&ctx, &package).await?;
            Ok(OperationResult::PackageInfo(info))
        }

        Commands::Search { query } => {
            let results = sps2_ops::search_packages(&ctx, &query).await?;
            Ok(OperationResult::SearchResults(results))
        }

        Commands::Cleanup => {
            let result = sps2_ops::cleanup(&ctx).await?;
            // Also update the GC timestamp through SystemSetup (best effort)
            if let Err(e) = crate::setup::SystemSetup::update_gc_timestamp_static().await {
                tracing::warn!("Failed to update GC timestamp: {}", e);
            }
            Ok(OperationResult::Success(result))
        }

        Commands::Rollback { state_id } => {
            let state_info = sps2_ops::rollback(&ctx, state_id).await?;
            Ok(OperationResult::StateInfo(state_info))
        }

        Commands::History { all, verify, limit } => {
            let history = sps2_ops::history(&ctx, all, verify, limit).await?;
            Ok(OperationResult::StateHistory(history))
        }

        Commands::CheckHealth => {
            let health = sps2_ops::check_health(&ctx).await?;
            Ok(OperationResult::HealthCheck(health))
        }

        // Large operations (delegate to specialized crates)
        Commands::Install { packages } => {
            let report = sps2_ops::install(&ctx, &packages).await?;
            Ok(OperationResult::InstallReport(report))
        }

        Commands::Update { packages } => {
            let report = sps2_ops::update(&ctx, &packages).await?;
            Ok(OperationResult::InstallReport(report))
        }

        Commands::Upgrade { packages } => {
            let report = sps2_ops::upgrade(&ctx, &packages).await?;
            Ok(OperationResult::InstallReport(report))
        }

        Commands::Uninstall { packages } => {
            let report = sps2_ops::uninstall(&ctx, &packages).await?;
            Ok(OperationResult::InstallReport(report))
        }

        Commands::Build {
            recipe,
            output_dir,
            network,
            jobs,
        } => {
            let output_path = output_dir.as_deref();
            let report = sps2_ops::build(&ctx, &recipe, output_path, network, jobs).await?;
            Ok(OperationResult::BuildReport(report))
        }

        Commands::Pack {
            recipe,
            directory,
            manifest,
            sbom,
            no_post,
            output_dir,
        } => {
            let output_path = output_dir.as_deref();
            let report = if let Some(dir) = directory {
                // The manifest is required with --directory, so we can unwrap it.
                let Some(manifest_path) = manifest else {
                    return Err(CliError::InvalidArguments(
                        "--manifest is required with --directory".to_string(),
                    ));
                };
                sps2_ops::pack_from_directory(
                    &ctx,
                    &dir,
                    &manifest_path,
                    sbom.as_deref(),
                    output_path,
                )
                .await?
            } else if let Some(rec) = recipe {
                if no_post {
                    sps2_ops::pack_from_recipe_no_post(&ctx, &rec, output_path).await?
                } else {
                    sps2_ops::pack_from_recipe(&ctx, &rec, output_path).await?
                }
            } else {
                // This case should be prevented by clap's arg group
                return Err(CliError::InvalidArguments(
                    "Either --recipe or --directory must be specified".to_string(),
                ));
            };
            Ok(OperationResult::BuildReport(report))
        }

        Commands::VulnDb { command } => match command {
            VulnDbCommands::Update => {
                let result = sps2_ops::update_vulndb(&ctx).await?;
                Ok(OperationResult::Success(result))
            }
            VulnDbCommands::Stats => {
                let stats = sps2_ops::vulndb_stats(&ctx).await?;
                Ok(OperationResult::VulnDbStats(stats))
            }
        },

        Commands::Audit {
            all: _,
            package,
            fail_on_critical,
            severity,
        } => {
            // Parse severity threshold
            let severity_threshold = match severity.as_deref() {
                Some("critical") => sps2_ops::Severity::Critical,
                Some("high") => sps2_ops::Severity::High,
                Some("medium") => sps2_ops::Severity::Medium,
                Some("low") | None => sps2_ops::Severity::Low,
                Some(s) => {
                    return Err(CliError::InvalidArguments(format!(
                        "Invalid severity '{s}': must be one of: low, medium, high, critical"
                    )))
                }
            };

            let report = sps2_ops::audit(
                &ctx,
                package.as_deref(),
                fail_on_critical,
                severity_threshold,
            )
            .await?;
            Ok(OperationResult::AuditReport(report))
        }

        Commands::SelfUpdate { skip_verify, force } => {
            let result = sps2_ops::self_update(&ctx, skip_verify, force).await?;
            Ok(OperationResult::Success(result))
        }

        Commands::Draft { source, output } => {
            sps2_ops::draft_recipe(
                &ctx,
                source.path,
                source.git,
                source.url,
                source.archive,
                output,
            )
            .await?;
            Ok(OperationResult::Success(
                "Recipe draft generated successfully".to_string(),
            ))
        }

        Commands::Verify {
            heal,
            level,
            scope,
            sync_refcounts,
        } => {
            let result = sps2_ops::verify(&ctx, heal, &level, &scope, sync_refcounts).await?;
            Ok(OperationResult::VerificationResult(result))
        }
    }
}

/// Build operations context with all required components
async fn build_ops_context(
    setup: &SystemSetup,
    event_sender: EventSender,
    config: Config,
    check_mode: bool,
) -> Result<sps2_ops::OpsCtx, CliError> {
    let ctx = OpsContextBuilder::new()
        .with_store(setup.store().clone())
        .with_state(setup.state().clone())
        .with_index(setup.index().clone())
        .with_net(setup.net().clone())
        .with_resolver(setup.resolver().clone())
        .with_builder(setup.builder())
        .with_event_sender(event_sender)
        .with_config(config)
        .with_check_mode(check_mode)
        .build()?;

    Ok(ctx)
}

/// Initialize tracing/logging
fn init_tracing(json_mode: bool, debug_enabled_flag: bool) {
    // Check if debug logging is enabled
    let debug_enabled = std::env::var("RUST_LOG").is_ok() || debug_enabled_flag;

    if json_mode {
        // JSON mode: suppress all console output to avoid contaminating JSON
        if debug_enabled {
            // In debug mode with JSON, still log to file
            let log_dir = std::path::Path::new(fixed_paths::LOGS_DIR);
            if std::fs::create_dir_all(log_dir).is_ok() {
                let log_file = log_dir.join(format!(
                    "sps2-{}.log",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                ));

                if let Ok(file) = std::fs::File::create(&log_file) {
                    tracing_subscriber::fmt()
                        .json()
                        .with_writer(file)
                        .with_env_filter(
                            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(
                                |_| {
                                    tracing_subscriber::EnvFilter::new(
                                        "info,sps2=debug,sps2_ops=info",
                                    )
                                },
                            ),
                        )
                        .init();
                    return;
                }
            }
        }
        // Fallback: disable all logging in JSON mode
        tracing_subscriber::fmt()
            .with_writer(std::io::sink)
            .with_env_filter("off")
            .init();
    } else if debug_enabled {
        // Debug mode: structured JSON logs to file
        let log_dir = std::path::Path::new(fixed_paths::LOGS_DIR);
        if let Err(e) = std::fs::create_dir_all(log_dir) {
            eprintln!("Warning: Failed to create log directory: {e}");
        }

        let log_file = log_dir.join(format!(
            "sps2-{}.log",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ));

        match std::fs::File::create(&log_file) {
            Ok(file) => {
                tracing_subscriber::fmt()
                    .json()
                    .with_writer(file)
                    .with_env_filter(
                        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(
                            |_| tracing_subscriber::EnvFilter::new("info,sps2=debug,sps2_ops=info"),
                        ),
                    )
                    .init();

                eprintln!("Debug logging enabled: {}", log_file.display());
            }
            Err(e) => {
                eprintln!("Warning: Failed to create log file: {e}");
                // Fallback to stderr
                tracing_subscriber::fmt()
                    .with_env_filter(
                        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(
                            |_| tracing_subscriber::EnvFilter::new("info,sps2=info,sps2_ops=info"),
                        ),
                    )
                    .init();
            }
        }
    } else {
        // Normal mode: minimal logging to stderr
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new("warn,sps2=warn,sps2_ops=warn")
                }),
            )
            .init();
    }
}

/// Show PATH reminder if needed
fn show_path_reminder_if_needed() {
    let path = std::env::var("PATH").unwrap_or_default();
    if !path.contains(fixed_paths::BIN_DIR) {
        eprintln!();
        eprintln!(
            "Add {} to your PATH to use installed packages:",
            fixed_paths::BIN_DIR
        );
        eprintln!(
            "   echo 'export PATH=\"{}:$PATH\"' >> ~/.zshrc",
            fixed_paths::BIN_DIR
        );
        eprintln!("   source ~/.zshrc");
        eprintln!();
    }
}

/// Apply CLI configuration overrides (highest precedence)
fn apply_cli_config(
    config: &mut Config,
    global: &cli::GlobalArgs,
    command: &cli::Commands,
) -> Result<(), CliError> {
    // Global CLI flags override everything
    if let Some(color) = &global.color {
        config.general.color = *color;
    }

    // Command-specific CLI flags
    if let cli::Commands::Build {
        jobs: Some(job_count),
        ..
    } = command
    {
        config.builder.build.build_jobs = *job_count;
    }

    Ok(())
}

/// Checks for and completes an interrupted transaction
async fn recover_if_needed(state_manager: &StateManager) -> Result<(), CliError> {
    if let Some(journal) = state_manager.read_journal().await? {
        warn!("Warning: A previous operation was interrupted. Attempting to recover...");

        match journal.phase {
            TransactionPhase::Prepared => {
                // The DB is prepared, filesystem step may or may not have completed.
                // New default behavior does an atomic rename of staging -> live.
                // If staging still exists, complete the rename; otherwise, assume it completed and just finalize DB.
                if sps2_platform::filesystem_helpers::exists(&journal.staging_path).await {
                    info!("Recovery: Completing filesystem finalize (rename + DB finalize)");
                    state_manager
                        .execute_filesystem_swap_and_finalize(journal)
                        .await?;
                } else {
                    info!("Recovery: Staging missing; assuming filesystem step completed. Finalizing DB state only.");
                    state_manager
                        .finalize_db_state(journal.new_state_id)
                        .await?;
                    state_manager.clear_journal().await?;
                }
            }
            TransactionPhase::Swapped => {
                // The FS swap happened, but the DB wasn't finalized.
                // We only need to finalize the DB state.
                info!("Recovery: Finalizing database state");
                state_manager
                    .finalize_db_state(journal.new_state_id)
                    .await?;
                state_manager.clear_journal().await?;
            }
        }
        warn!("Recovery successful.");
    }
    Ok(())
}

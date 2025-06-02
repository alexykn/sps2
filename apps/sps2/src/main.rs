//! sps2 - Modern package manager for macOS ARM64
//!
//! This is the main CLI application that orchestrates all package management
//! operations through the ops crate.

mod cli;
mod display;
mod error;
mod events;
mod setup;

use crate::cli::{Cli, Commands, VulnDbCommands};
use crate::display::OutputRenderer;
use crate::error::CliError;
use crate::events::EventHandler;
use crate::setup::SystemSetup;
use clap::Parser;
use sps2_config::Config;
use sps2_events::{EventReceiver, EventSender};
use sps2_ops::{OperationResult, OpsContextBuilder};
use std::process;
use tokio::select;
use tracing::{error, info};

#[tokio::main]
async fn main() {
    // Parse command line arguments first to check for JSON mode
    let cli = Cli::parse();
    let json_mode = cli.global.json;

    // Initialize tracing with JSON awareness
    init_tracing(json_mode);

    // Run the application and handle errors
    if let Err(e) = run(cli).await {
        error!("Application error: {}", e);
        if !json_mode {
            eprintln!("Error: {}", e);
        }
        process::exit(1);
    }
}

/// Main application logic
async fn run(cli: Cli) -> Result<(), CliError> {
    info!("Starting sps2 v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration with proper precedence:
    // 1. Start with file config (or defaults)
    let mut config = Config::load_or_default(&cli.global.config).await?;

    // 2. Merge environment variables
    config.merge_env()?;

    // 3. Apply CLI flags (highest precedence)
    apply_cli_config(&mut config, &cli.global, &cli.command)?;

    // Initialize system setup
    let mut setup = SystemSetup::new(config.clone());

    // Perform startup checks and initialization
    setup.initialize().await?;

    // Create event channel
    let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

    // Build operations context
    let ops_ctx = build_ops_context(&setup, event_sender.clone()).await?;

    // Create output renderer
    let renderer = OutputRenderer::new(
        cli.global.json,
        cli.global.color.unwrap_or(config.general.color),
    );

    // Create event handler
    let mut event_handler = EventHandler::new(renderer.clone());

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
                    None => break, // Channel closed
                }
            }
        }
    }

    Err(CliError::EventChannelClosed)
}

/// Execute the specified command
async fn execute_command(
    command: Commands,
    ctx: sps2_ops::OpsCtx,
) -> Result<OperationResult, CliError> {
    match command {
        // Small operations (implemented in ops crate)
        Commands::Reposync => {
            let result = sps2_ops::reposync(&ctx).await?;
            Ok(OperationResult::Success(result))
        }

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

        Commands::History => {
            let history = sps2_ops::history(&ctx).await?;
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
            compression_level: _,
            fast: _,
            max: _,
            legacy: _,
        } => {
            let output_path = output_dir.as_deref();
            let report = sps2_ops::build(&ctx, &recipe, output_path, network, jobs).await?;
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
                        "Invalid severity '{}': must be one of: low, medium, high, critical",
                        s
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
    }
}

/// Build operations context with all required components
async fn build_ops_context(
    setup: &SystemSetup,
    event_sender: EventSender,
) -> Result<sps2_ops::OpsCtx, CliError> {
    let ctx = OpsContextBuilder::new()
        .with_store(setup.store().clone())
        .with_state(setup.state().clone())
        .with_index(setup.index().clone())
        .with_net(setup.net().clone())
        .with_resolver(setup.resolver().clone())
        .with_builder(setup.builder())
        .with_event_sender(event_sender)
        .build()?;

    Ok(ctx)
}

/// Initialize tracing/logging
fn init_tracing(json_mode: bool) {
    // Check if debug logging is enabled
    let debug_enabled =
        std::env::var("RUST_LOG").is_ok() || std::env::args().any(|arg| arg == "--debug");

    if json_mode {
        // JSON mode: suppress all console output to avoid contaminating JSON
        if debug_enabled {
            // In debug mode with JSON, still log to file
            let log_dir = std::path::Path::new("/opt/pm/logs");
            if std::fs::create_dir_all(log_dir).is_ok() {
                let log_file = log_dir.join(format!(
                    "sps2-{}.log",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                ));

                if let Ok(file) = std::fs::File::create(&log_file) {
                    tracing_subscriber::fmt()
                        .json()
                        .with_writer(file)
                        .with_env_filter("sps2=debug,sps2=debug")
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
        let log_dir = std::path::Path::new("/opt/pm/logs");
        if let Err(e) = std::fs::create_dir_all(log_dir) {
            eprintln!("Warning: Failed to create log directory: {}", e);
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
                    .with_env_filter("sps2=debug,sps2=debug")
                    .init();

                eprintln!("Debug logging enabled: {}", log_file.display());
            }
            Err(e) => {
                eprintln!("Warning: Failed to create log file: {}", e);
                // Fallback to stderr
                tracing_subscriber::fmt()
                    .with_env_filter("sps2=info,sps2=info")
                    .init();
            }
        }
    } else {
        // Normal mode: minimal logging to stderr
        tracing_subscriber::fmt()
            .with_env_filter("sps2=warn,sps2=warn")
            .init();
    }
}

/// Show PATH reminder if needed
fn show_path_reminder_if_needed() {
    let path = std::env::var("PATH").unwrap_or_default();
    if !path.contains("/opt/pm/live/bin") {
        eprintln!();
        eprintln!("Add /opt/pm/live/bin to your PATH to use installed packages:");
        eprintln!("   echo 'export PATH=\"/opt/pm/live/bin:$PATH\"' >> ~/.zshrc");
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
    match command {
        cli::Commands::Build { network, jobs, .. } => {
            if *network {
                config.build.network_access = true;
            }
            if let Some(job_count) = jobs {
                config.build.build_jobs = *job_count;
            }
        }
        _ => {
            // No command-specific config overrides for other commands yet
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_types::ColorChoice;

    #[test]
    fn test_cli_config_precedence() {
        let mut config = Config::default();

        // Start with default (auto color)
        assert_eq!(config.general.color, ColorChoice::Auto);

        // Test CLI override
        let global_args = cli::GlobalArgs {
            json: false,
            debug: false,
            color: Some(ColorChoice::Always),
            config: None,
        };

        let command = cli::Commands::List;

        apply_cli_config(&mut config, &global_args, &command).unwrap();

        // CLI should override
        assert_eq!(config.general.color, ColorChoice::Always);
    }

    #[test]
    fn test_build_command_config_override() {
        let mut config = Config::default();

        // Start with defaults
        assert!(!config.build.network_access);
        assert_eq!(config.build.build_jobs, 0);

        let global_args = cli::GlobalArgs {
            json: false,
            debug: false,
            color: None,
            config: None,
        };

        let command = cli::Commands::Build {
            recipe: std::path::PathBuf::from("test.star"),
            output_dir: None,
            network: true,
            jobs: Some(8),
            compression_level: None,
            fast: false,
            max: false,
            legacy: false,
        };

        apply_cli_config(&mut config, &global_args, &command).unwrap();

        // Build command should override config
        assert!(config.build.network_access);
        assert_eq!(config.build.build_jobs, 8);
    }
}

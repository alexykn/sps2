//! sps2 - Modern package manager for macOS ARM64
//!
//! This is the main CLI application that orchestrates all package management
//! operations through the ops crate.

mod cli;
mod display;
mod error;
mod events;
mod setup;

use crate::cli::{Cli, Commands};
use crate::display::OutputRenderer;
use crate::error::CliError;
use crate::events::EventHandler;
use crate::setup::SystemSetup;
use clap::Parser;
use spsv2_config::Config;
use spsv2_events::{Event, EventReceiver, EventSender};
use spsv2_ops::{OperationResult, OpsContextBuilder};
use std::process;
use tokio::select;
use tracing::{error, info};

#[tokio::main]
async fn main() {
    // Initialize tracing first
    init_tracing();

    // Parse command line arguments
    let cli = Cli::parse();

    // Run the application and handle errors
    if let Err(e) = run(cli).await {
        error!("Application error: {}", e);
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

/// Main application logic
async fn run(cli: Cli) -> Result<(), CliError> {
    info!("Starting sps2 v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = Config::load_or_default(&cli.global.config).await?;

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
    ops_ctx: spsv2_ops::OpsCtx,
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
    ctx: spsv2_ops::OpsCtx,
) -> Result<OperationResult, CliError> {
    match command {
        // Small operations (implemented in ops crate)
        Commands::Reposync => {
            let result = spsv2_ops::reposync(&ctx).await?;
            Ok(OperationResult::Success(result))
        }

        Commands::List => {
            let packages = spsv2_ops::list_packages(&ctx).await?;
            Ok(OperationResult::PackageList(packages))
        }

        Commands::Info { package } => {
            let info = spsv2_ops::package_info(&ctx, &package).await?;
            Ok(OperationResult::PackageInfo(info))
        }

        Commands::Search { query } => {
            let results = spsv2_ops::search_packages(&ctx, &query).await?;
            Ok(OperationResult::SearchResults(results))
        }

        Commands::Cleanup => {
            let result = spsv2_ops::cleanup(&ctx).await?;
            Ok(OperationResult::Success(result))
        }

        Commands::Rollback { state_id } => {
            let state_info = spsv2_ops::rollback(&ctx, state_id).await?;
            Ok(OperationResult::StateInfo(state_info))
        }

        Commands::History => {
            let history = spsv2_ops::history(&ctx).await?;
            Ok(OperationResult::StateHistory(history))
        }

        Commands::CheckHealth => {
            let health = spsv2_ops::check_health(&ctx).await?;
            Ok(OperationResult::HealthCheck(health))
        }

        // Large operations (delegate to specialized crates)
        Commands::Install { packages } => {
            let report = spsv2_ops::install(&ctx, &packages).await?;
            Ok(OperationResult::InstallReport(report))
        }

        Commands::Update { packages } => {
            let report = spsv2_ops::update(&ctx, &packages).await?;
            Ok(OperationResult::InstallReport(report))
        }

        Commands::Upgrade { packages } => {
            let report = spsv2_ops::upgrade(&ctx, &packages).await?;
            Ok(OperationResult::InstallReport(report))
        }

        Commands::Uninstall { packages } => {
            let report = spsv2_ops::uninstall(&ctx, &packages).await?;
            Ok(OperationResult::InstallReport(report))
        }

        Commands::Build {
            recipe,
            output_dir,
            network,
            jobs: _,
        } => {
            // TODO: Pass network and jobs options to builder
            let output_path = output_dir.as_deref();
            let report = spsv2_ops::build(&ctx, &recipe, output_path).await?;
            Ok(OperationResult::BuildReport(report))
        }
    }
}

/// Build operations context with all required components
async fn build_ops_context(
    setup: &SystemSetup,
    event_sender: EventSender,
) -> Result<spsv2_ops::OpsCtx, CliError> {
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
fn init_tracing() {
    // Check if debug logging is enabled
    let debug_enabled =
        std::env::var("RUST_LOG").is_ok() || std::env::args().any(|arg| arg == "--debug");

    if debug_enabled {
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
                    .with_env_filter("sps2=debug,spsv2=debug")
                    .init();

                eprintln!("Debug logging enabled: {}", log_file.display());
            }
            Err(e) => {
                eprintln!("Warning: Failed to create log file: {}", e);
                // Fallback to stderr
                tracing_subscriber::fmt()
                    .with_env_filter("sps2=info,spsv2=info")
                    .init();
            }
        }
    } else {
        // Normal mode: minimal logging to stderr
        tracing_subscriber::fmt()
            .with_env_filter("sps2=warn,spsv2=warn")
            .init();
    }
}

/// Show PATH reminder if needed
fn show_path_reminder_if_needed() {
    let path = std::env::var("PATH").unwrap_or_default();
    if !path.contains("/opt/pm/live/bin") {
        eprintln!();
        eprintln!("📝 Add /opt/pm/live/bin to your PATH to use installed packages:");
        eprintln!("   echo 'export PATH=\"/opt/pm/live/bin:$PATH\"' >> ~/.zshrc");
        eprintln!("   source ~/.zshrc");
        eprintln!();
    }
}

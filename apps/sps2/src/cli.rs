//! Command line interface definition

use clap::{Parser, Subcommand};
use sps2_types::ColorChoice;
use std::path::PathBuf;
use uuid::Uuid;

/// sps2 - Modern package manager for macOS ARM64
#[derive(Parser)]
#[command(name = "sps2")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Modern package manager for macOS ARM64")]
#[command(long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[command(flatten)]
    pub global: GlobalArgs,
}

/// Global arguments available for all commands
#[derive(Parser)]
pub struct GlobalArgs {
    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,

    /// Enable debug logging to /opt/pm/logs/
    #[arg(long, global = true)]
    pub debug: bool,

    /// Color output control
    #[arg(long, global = true, value_enum)]
    pub color: Option<ColorChoice>,

    /// Use alternate config file
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

/// Draft command source arguments
#[derive(Debug, Parser)]
#[clap(group(
    clap::ArgGroup::new("source")
        .required(true)
        .args(&["path", "git", "url", "archive"]),
))]
pub struct DraftSource {
    /// Local source code directory
    #[clap(long, short = 'p', value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Git repository URL
    #[clap(long, short = 'g', value_name = "URL")]
    pub git: Option<String>,

    /// Direct URL to source archive
    #[clap(long, short = 'u', value_name = "URL")]
    pub url: Option<String>,

    /// Local archive file
    #[clap(long, short = 'a', value_name = "PATH")]
    pub archive: Option<PathBuf>,
}

/// Available commands
#[derive(Subcommand)]
pub enum Commands {
    /// Install packages from repository or local files
    #[command(alias = "i")]
    Install {
        /// Package specifications (name, name>=version, or ./file.sp)
        packages: Vec<String>,
    },

    /// Update packages to newer compatible versions
    #[command(alias = "up")]
    Update {
        /// Specific packages to update (empty = all packages)
        packages: Vec<String>,
    },

    /// Upgrade packages to latest versions (ignore upper bounds)
    #[command(alias = "ug")]
    Upgrade {
        /// Specific packages to upgrade (empty = all packages)
        packages: Vec<String>,
    },

    /// Uninstall packages
    #[command(alias = "rm")]
    Uninstall {
        /// Package names to uninstall
        packages: Vec<String>,
    },

    /// Build package from Starlark recipe
    Build {
        /// Path to recipe file (.star)
        recipe: PathBuf,

        /// Output directory for .sp file
        #[arg(short, long)]
        output_dir: Option<PathBuf>,

        /// Allow network access during build
        #[arg(long)]
        network: bool,

        /// Number of parallel build jobs (0=auto)
        #[arg(short, long)]
        jobs: Option<usize>,

        /// Compression level: fast, balanced, maximum, or 1-22
        #[arg(long, value_name = "LEVEL")]
        compression_level: Option<String>,

        /// Use fast compression (equivalent to --compression-level fast)
        #[arg(long, conflicts_with = "compression_level")]
        fast: bool,

        /// Use maximum compression (equivalent to --compression-level maximum)
        #[arg(long, conflicts_with_all = ["compression_level", "fast"])]
        max: bool,

        /// Use legacy compression format (single stream, no seeking)
        #[arg(long)]
        legacy: bool,
    },

    /// List installed packages
    #[command(alias = "ls")]
    List,

    /// Show information about a package
    Info {
        /// Package name
        package: String,
    },

    /// Search for packages
    #[command(alias = "find")]
    Search {
        /// Search query
        query: String,
    },

    /// Sync repository index
    #[command(alias = "sync")]
    Reposync,

    /// Clean up orphaned packages and old states
    Cleanup,

    /// Rollback to previous state
    Rollback {
        /// Target state ID (empty = previous state)
        state_id: Option<Uuid>,
    },

    /// Show state history
    History,

    /// Check system health
    #[command(name = "check-health")]
    CheckHealth,

    /// Vulnerability database management
    #[command(name = "vulndb")]
    VulnDb {
        #[command(subcommand)]
        command: VulnDbCommands,
    },

    /// Draft a new build recipe from a source
    Draft {
        #[command(flatten)]
        source: DraftSource,

        /// Path to save the generated recipe (defaults to './<name>-<version>.star')
        #[clap(short, long)]
        output: Option<PathBuf>,
    },

    /// Audit installed packages for vulnerabilities
    Audit {
        /// Scan all packages (default: all)
        #[arg(long)]
        all: bool,

        /// Scan specific package
        #[arg(long, value_name = "NAME")]
        package: Option<String>,

        /// Fail on critical vulnerabilities
        #[arg(long)]
        fail_on_critical: bool,

        /// Minimum severity to report (low, medium, high, critical)
        #[arg(long, value_name = "SEVERITY")]
        severity: Option<String>,
    },

    /// Update sps2 to the latest version
    #[command(name = "self-update")]
    SelfUpdate {
        /// Skip signature verification (not recommended)
        #[arg(long)]
        skip_verify: bool,

        /// Force update even if already on latest version
        #[arg(long)]
        force: bool,
    },

    /// Verify and optionally heal the current state
    Verify {
        /// Automatically heal any discrepancies found
        #[arg(long)]
        heal: bool,

        /// Verification level (quick, standard, full)
        #[arg(long, default_value = "standard")]
        level: String,
    },

    /// Explore the content-addressed store
    Store {
        #[command(subcommand)]
        command: StoreCommands,
    },
}

/// Vulnerability database subcommands
#[derive(Subcommand)]
pub enum VulnDbCommands {
    /// Update vulnerability database from sources
    Update,

    /// Show vulnerability database statistics
    Stats,
}

impl Commands {
    /// Extract compression configuration from build command arguments
    ///
    /// Returns a tuple of (compression_level_string, use_legacy_format)
    #[allow(dead_code)] // Used in tests, will be used when compression config is wired up
    pub fn compression_config(&self) -> Option<(String, bool)> {
        match self {
            Commands::Build {
                compression_level,
                fast,
                max,
                legacy,
                ..
            } => {
                let level = if *fast {
                    "fast".to_string()
                } else if *max {
                    "maximum".to_string()
                } else if let Some(level) = compression_level {
                    level.clone()
                } else {
                    "balanced".to_string() // Default
                };
                Some((level, *legacy))
            }
            _ => None,
        }
    }

    /// Get command name for logging
    #[allow(dead_code)] // Used in tests
    pub fn name(&self) -> &'static str {
        match self {
            Commands::Install { .. } => "install",
            Commands::Update { .. } => "update",
            Commands::Upgrade { .. } => "upgrade",
            Commands::Uninstall { .. } => "uninstall",
            Commands::Build { .. } => "build",
            Commands::List => "list",
            Commands::Info { .. } => "info",
            Commands::Search { .. } => "search",
            Commands::Reposync => "reposync",
            Commands::Cleanup => "cleanup",
            Commands::Rollback { .. } => "rollback",
            Commands::History => "history",
            Commands::CheckHealth => "check-health",
            Commands::VulnDb { .. } => "vulndb",
            Commands::Audit { .. } => "audit",
            Commands::SelfUpdate { .. } => "self-update",
            Commands::Draft { .. } => "draft",
            Commands::Verify { .. } => "verify",
        }
    }

    /// Validate command arguments
    #[allow(dead_code)] // Used in tests
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Commands::Install { packages } if packages.is_empty() => {
                Err("No packages specified for installation".to_string())
            }
            Commands::Uninstall { packages } if packages.is_empty() => {
                Err("No packages specified for removal".to_string())
            }
            Commands::Build {
                recipe,
                compression_level,
                ..
            } => {
                if !recipe.exists() {
                    Err(format!("Recipe file not found: {}", recipe.display()))
                } else if recipe.extension().is_none_or(|ext| ext != "star") {
                    Err("Recipe file must have .star extension".to_string())
                } else if let Some(level) = compression_level {
                    // Validate compression level
                    match level.to_lowercase().as_str() {
                        "fast" | "balanced" | "maximum" | "max" => Ok(()),
                        numeric => {
                            if let Ok(level_num) = numeric.parse::<u8>() {
                                if (1..=22).contains(&level_num) {
                                    Ok(())
                                } else {
                                    Err(format!("Compression level must be between 1 and 22, got {level_num}"))
                                }
                            } else {
                                Err(format!(
                                    "Invalid compression level '{}'. Valid options: fast, balanced, maximum, or 1-22",
                                    level
                                ))
                            }
                        }
                    }
                } else {
                    Ok(())
                }
            }
            Commands::Info { package } if package.is_empty() => {
                Err("Package name cannot be empty".to_string())
            }
            Commands::Search { query } if query.is_empty() => {
                Err("Search query cannot be empty".to_string())
            }
            _ => Ok(()),
        }
    }
}

//! Command line interface definition

use clap::{Parser, Subcommand, ValueEnum};
use spsv2_types::ColorChoice;
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

    /// Build package from Rhai recipe
    Build {
        /// Path to recipe file (.rhai)
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
}

impl Commands {
    /// Get command name for logging
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
        }
    }

    /// Check if command requires package arguments
    pub fn requires_packages(&self) -> bool {
        matches!(self,
            Commands::Install { packages } |
            Commands::Uninstall { packages } if packages.is_empty()
        )
    }

    /// Validate command arguments
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Commands::Install { packages } if packages.is_empty() => {
                Err("No packages specified for installation".to_string())
            }
            Commands::Uninstall { packages } if packages.is_empty() => {
                Err("No packages specified for removal".to_string())
            }
            Commands::Build { recipe, .. } => {
                if !recipe.exists() {
                    Err(format!("Recipe file not found: {}", recipe.display()))
                } else if !recipe.extension().map_or(false, |ext| ext == "rhai") {
                    Err("Recipe file must have .rhai extension".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_parsing() {
        // Test basic install command
        let cli = Cli::parse_from(&["sps2", "install", "curl"]);
        assert!(matches!(cli.command, Commands::Install { .. }));

        // Test install with version constraints
        let cli = Cli::parse_from(&["sps2", "install", "curl>=8.0.0"]);
        if let Commands::Install { packages } = cli.command {
            assert_eq!(packages, vec!["curl>=8.0.0"]);
        } else {
            panic!("Expected Install command");
        }

        // Test global flags
        let cli = Cli::parse_from(&["sps2", "--json", "--debug", "list"]);
        assert!(cli.global.json);
        assert!(cli.global.debug);
        assert!(matches!(cli.command, Commands::List));
    }

    #[test]
    fn test_command_aliases() {
        // Test install alias
        let cli = Cli::parse_from(&["sps2", "i", "curl"]);
        assert!(matches!(cli.command, Commands::Install { .. }));

        // Test update alias
        let cli = Cli::parse_from(&["sps2", "up"]);
        assert!(matches!(cli.command, Commands::Update { .. }));

        // Test list alias
        let cli = Cli::parse_from(&["sps2", "ls"]);
        assert!(matches!(cli.command, Commands::List));
    }

    #[test]
    fn test_command_validation() {
        let install_empty = Commands::Install { packages: vec![] };
        assert!(install_empty.validate().is_err());

        let install_valid = Commands::Install {
            packages: vec!["curl".to_string()],
        };
        assert!(install_valid.validate().is_ok());

        let info_empty = Commands::Info {
            package: String::new(),
        };
        assert!(info_empty.validate().is_err());

        let info_valid = Commands::Info {
            package: "curl".to_string(),
        };
        assert!(info_valid.validate().is_ok());
    }

    #[test]
    fn test_command_names() {
        assert_eq!(Commands::Install { packages: vec![] }.name(), "install");
        assert_eq!(Commands::List.name(), "list");
        assert_eq!(Commands::CheckHealth.name(), "check-health");
    }
}

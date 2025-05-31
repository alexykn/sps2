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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_parsing() {
        // Test basic install command
        let cli = Cli::parse_from(["sps2", "install", "curl"]);
        assert!(matches!(cli.command, Commands::Install { .. }));

        // Test install with version constraints
        let cli = Cli::parse_from(["sps2", "install", "curl>=8.0.0"]);
        if let Commands::Install { packages } = cli.command {
            assert_eq!(packages, vec!["curl>=8.0.0"]);
        } else {
            panic!("Expected Install command");
        }

        // Test global flags
        let cli = Cli::parse_from(["sps2", "--json", "--debug", "list"]);
        assert!(cli.global.json);
        assert!(cli.global.debug);
        assert!(matches!(cli.command, Commands::List));
    }

    #[test]
    fn test_command_aliases() {
        // Test install alias
        let cli = Cli::parse_from(["sps2", "i", "curl"]);
        assert!(matches!(cli.command, Commands::Install { .. }));

        // Test update alias
        let cli = Cli::parse_from(["sps2", "up"]);
        assert!(matches!(cli.command, Commands::Update { .. }));

        // Test list alias
        let cli = Cli::parse_from(["sps2", "ls"]);
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

    #[test]
    fn test_build_compression_flags() {
        // Test --fast flag
        let cli = Cli::parse_from(["sps2", "build", "--fast", "test.star"]);
        if let Commands::Build {
            fast,
            max,
            compression_level,
            legacy,
            ..
        } = cli.command
        {
            assert!(fast);
            assert!(!max);
            assert!(compression_level.is_none());
            assert!(!legacy);
        } else {
            panic!("Expected Build command");
        }

        // Test --max flag
        let cli = Cli::parse_from(["sps2", "build", "--max", "test.star"]);
        if let Commands::Build {
            fast,
            max,
            compression_level,
            legacy,
            ..
        } = cli.command
        {
            assert!(!fast);
            assert!(max);
            assert!(compression_level.is_none());
            assert!(!legacy);
        } else {
            panic!("Expected Build command");
        }

        // Test --compression-level flag
        let cli = Cli::parse_from(["sps2", "build", "--compression-level", "15", "test.star"]);
        if let Commands::Build {
            fast,
            max,
            compression_level,
            legacy,
            ..
        } = cli.command
        {
            assert!(!fast);
            assert!(!max);
            assert_eq!(compression_level.as_deref(), Some("15"));
            assert!(!legacy);
        } else {
            panic!("Expected Build command");
        }

        // Test --legacy flag
        let cli = Cli::parse_from(["sps2", "build", "--legacy", "test.star"]);
        if let Commands::Build { legacy, .. } = cli.command {
            assert!(legacy);
        } else {
            panic!("Expected Build command");
        }
    }

    #[test]
    fn test_compression_config_extraction() {
        // Test fast compression
        let build_fast = Commands::Build {
            recipe: PathBuf::from("test.star"),
            output_dir: None,
            network: false,
            jobs: None,
            compression_level: None,
            fast: true,
            max: false,
            legacy: false,
        };
        let (level, legacy) = build_fast.compression_config().unwrap();
        assert_eq!(level, "fast");
        assert!(!legacy);

        // Test maximum compression
        let build_max = Commands::Build {
            recipe: PathBuf::from("test.star"),
            output_dir: None,
            network: false,
            jobs: None,
            compression_level: None,
            fast: false,
            max: true,
            legacy: true,
        };
        let (level, legacy) = build_max.compression_config().unwrap();
        assert_eq!(level, "maximum");
        assert!(legacy);

        // Test custom level
        let build_custom = Commands::Build {
            recipe: PathBuf::from("test.star"),
            output_dir: None,
            network: false,
            jobs: None,
            compression_level: Some("15".to_string()),
            fast: false,
            max: false,
            legacy: false,
        };
        let (level, legacy) = build_custom.compression_config().unwrap();
        assert_eq!(level, "15");
        assert!(!legacy);

        // Test default (balanced)
        let build_default = Commands::Build {
            recipe: PathBuf::from("test.star"),
            output_dir: None,
            network: false,
            jobs: None,
            compression_level: None,
            fast: false,
            max: false,
            legacy: false,
        };
        let (level, legacy) = build_default.compression_config().unwrap();
        assert_eq!(level, "balanced");
        assert!(!legacy);

        // Test non-build command
        let list = Commands::List;
        assert!(list.compression_config().is_none());
    }

    #[test]
    fn test_compression_level_validation() {
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let recipe_path = temp.path().join("test.star");
        std::fs::write(&recipe_path, "# test recipe").unwrap();

        // Test valid compression levels
        let build_valid = Commands::Build {
            recipe: recipe_path.clone(),
            output_dir: None,
            network: false,
            jobs: None,
            compression_level: Some("fast".to_string()),
            fast: false,
            max: false,
            legacy: false,
        };
        assert!(build_valid.validate().is_ok());

        let build_numeric = Commands::Build {
            recipe: recipe_path.clone(),
            output_dir: None,
            network: false,
            jobs: None,
            compression_level: Some("15".to_string()),
            fast: false,
            max: false,
            legacy: false,
        };
        assert!(build_numeric.validate().is_ok());

        // Test invalid compression levels
        let build_invalid = Commands::Build {
            recipe: recipe_path.clone(),
            output_dir: None,
            network: false,
            jobs: None,
            compression_level: Some("invalid".to_string()),
            fast: false,
            max: false,
            legacy: false,
        };
        assert!(build_invalid.validate().is_err());

        let build_out_of_range = Commands::Build {
            recipe: recipe_path,
            output_dir: None,
            network: false,
            jobs: None,
            compression_level: Some("25".to_string()),
            fast: false,
            max: false,
            legacy: false,
        };
        assert!(build_out_of_range.validate().is_err());
    }
}

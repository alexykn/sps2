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
#[derive(clap::Args)]
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

    /// Use alternate builder config file
    #[arg(long, global = true, value_name = "PATH")]
    pub builder_config: Option<PathBuf>,

    /// Show what would be done without executing (like ansible --check)
    #[arg(long, global = true)]
    pub check: bool,
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

    /// Build package from YAML recipe
    Build {
        /// Path to recipe file (.yaml or .yml)
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

    /// Package from staging directory without rebuilding
    #[command(alias = "p")]
    #[command(group(
        clap::ArgGroup::new("pack_source")
            .required(true)
            .args(&["recipe", "directory"]),
    ))]
    #[command(group(
        clap::ArgGroup::new("dir_requires_manifest")
            .args(&["directory", "manifest"]) 
            .requires_all(&["directory", "manifest"]),
    ))]
    Pack {
        /// Path to recipe file (.yaml or .yml)
        #[arg(short = 'r', long = "recipe")]
        recipe: Option<PathBuf>,

        /// Path to a directory to package directly (skips post-processing)
        #[arg(short = 'd', long = "directory")]
        directory: Option<PathBuf>,

        /// Path to a manifest.toml file (required with --directory)
        #[arg(short = 'm', long, requires = "directory")]
        manifest: Option<PathBuf>,

        /// Path to an SBOM file (optional, requires --directory)
        #[arg(short = 's', long, requires = "directory")]
        sbom: Option<PathBuf>,

        /// Skip post-processing steps and QA pipeline (only with --recipe)
        #[arg(short = 'n', long = "no-post", requires = "recipe")]
        no_post: bool,

        /// Output directory for .sp file
        #[arg(short, long)]
        output_dir: Option<PathBuf>,
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
    Reposync {
        /// Automatically trust new keys
        #[clap(long)]
        yes: bool,
    },

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

        /// Verification scope (live, store, all)
        #[arg(long, default_value = "live")]
        scope: String,
    },

    /// Manage repositories
    #[command(subcommand)]
    Repo(RepoCommands),

    /// Manage trusted signing keys
    #[command(subcommand)]
    Keys(KeysCommands),
}

/// Repository management subcommands
#[derive(Subcommand)]
pub enum RepoCommands {
    /// Add a new repository
    Add {
        /// A unique name for the repository
        #[clap(required = true)]
        name: String,
        /// The URL of the repository
        #[clap(required = true)]
        url: String,
    },

    /// List configured repositories
    List,

    /// Remove a repository by name
    Remove {
        /// Repository name (e.g., fast, slow, stable, or extras key)
        name: String,
    },
}

/// Key management subcommands
#[derive(Subcommand)]
pub enum KeysCommands {
    /// List trusted signing keys
    List,

    /// Import a Minisign public key (.pub or base64)
    Import {
        /// Path to Minisign public key (.pub or text file containing base64)
        file: PathBuf,
        /// Optional comment to store with the key
        #[arg(long)]
        comment: Option<String>,
    },

    /// Remove a trusted key by key ID
    Remove {
        /// Minisign key ID (hex)
        key_id: String,
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

impl Commands {}

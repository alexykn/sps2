//! Types and result structures for build environment

use std::fmt;
use std::path::PathBuf;

/// Result of executing a build command
#[derive(Debug)]
pub struct BuildCommandResult {
    /// Whether the command succeeded
    pub success: bool,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
}

/// Result of the build process
#[derive(Debug, Clone)]
pub struct BuildResult {
    /// Path to the generated package file
    pub package_path: PathBuf,
    /// SBOM files generated
    pub sbom_files: Vec<PathBuf>,
    /// Build log
    pub build_log: String,
    /// Whether the recipe requested the package be installed after building
    pub install_requested: bool,
}

impl BuildResult {
    /// Create new build result
    #[must_use]
    pub fn new(package_path: PathBuf) -> Self {
        Self {
            package_path,
            sbom_files: Vec::new(),
            build_log: String::new(),
            install_requested: false,
        }
    }

    /// Add SBOM file
    pub fn add_sbom_file(&mut self, path: PathBuf) {
        self.sbom_files.push(path);
    }

    /// Set build log
    pub fn set_build_log(&mut self, log: String) {
        self.build_log = log;
    }

    /// Set install requested flag
    #[must_use]
    pub fn with_install_requested(mut self, install_requested: bool) -> Self {
        self.install_requested = install_requested;
        self
    }
}

/// Build isolation level
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IsolationLevel {
    /// No isolation - uses host environment as-is (shows warning)
    None = 0,
    /// Default isolation - clean environment, controlled paths (default)
    Default = 1,
    /// Enhanced isolation - default + private HOME/TMPDIR
    Enhanced = 2,
    /// Hermetic isolation - full whitelist approach, network blocking
    Hermetic = 3,
}

impl Default for IsolationLevel {
    fn default() -> Self {
        Self::Default
    }
}

impl IsolationLevel {
    /// Convert from u8
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Default),
            2 => Some(Self::Enhanced),
            3 => Some(Self::Hermetic),
            _ => None,
        }
    }

    /// Convert to u8
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Default => write!(f, "default"),
            Self::Enhanced => write!(f, "enhanced"),
            Self::Hermetic => write!(f, "hermetic"),
        }
    }
}

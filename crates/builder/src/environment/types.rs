//! Types and result structures for build environment

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
}

impl BuildResult {
    /// Create new build result
    #[must_use]
    pub fn new(package_path: PathBuf) -> Self {
        Self {
            package_path,
            sbom_files: Vec::new(),
            build_log: String::new(),
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
}

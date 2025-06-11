//! Builder API for Starlark recipes

use crate::{BuildCommandResult, BuildEnvironment};
use sps2_errors::{BuildError, Error};
use sps2_hash::Hash;
use sps2_net::{NetClient, NetConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Builder API exposed to Starlark recipes
#[derive(Clone)]
pub struct BuilderApi {
    /// Working directory for source extraction
    working_dir: PathBuf,
    /// Downloaded files
    downloads: HashMap<String, PathBuf>,
    /// Network client for downloads
    net_client: NetClient,
    /// Whether network access is allowed
    allow_network: bool,
    /// SBOM generation enabled
    auto_sbom: bool,
    /// SBOM exclusion patterns
    sbom_excludes: Vec<String>,
    /// Whether install was requested during recipe execution
    install_requested: bool,
    /// Build metadata collected during build (e.g., Python wheel path)
    build_metadata: HashMap<String, String>,
}

impl BuilderApi {
    /// Create new builder API
    ///
    /// # Errors
    ///
    /// Returns an error if the network client cannot be created.
    pub fn new(working_dir: PathBuf) -> Result<Self, Error> {
        Ok(Self {
            working_dir,
            downloads: HashMap::new(),
            net_client: NetClient::new(NetConfig::default())?,
            allow_network: false,
            auto_sbom: true,
            sbom_excludes: vec![
                "./*.dSYM".to_string(),
                "./*.pdb".to_string(),
                "./*.a".to_string(),
                "./*.la".to_string(),
            ],
            install_requested: false,
            build_metadata: HashMap::new(),
        })
    }

    /// Allow network access during build
    #[must_use]
    pub fn allow_network(&mut self, allow: bool) -> &mut Self {
        self.allow_network = allow;
        self
    }

    /// Download and verify a file
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Network access is disabled
    /// - The URL is invalid
    /// - The download fails
    /// - The file hash doesn't match the expected hash
    pub async fn fetch(&mut self, url: &str, expected_hash: &str) -> Result<PathBuf, Error> {
        if !self.allow_network {
            return Err(BuildError::NetworkDisabled {
                url: url.to_string(),
            }
            .into());
        }

        // Check if already downloaded
        if let Some(path) = self.downloads.get(url) {
            return Ok(path.clone());
        }

        // Extract filename from URL
        let filename = url
            .split('/')
            .next_back()
            .ok_or_else(|| BuildError::InvalidUrl {
                url: url.to_string(),
            })?;

        let download_path = self.working_dir.join(filename);

        // Download file using the download module
        // For builder, we don't have an event sender, so we'll use the client directly
        let response = self.net_client.get(url).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|_e| BuildError::FetchFailed {
                url: url.to_string(),
            })?;
        fs::write(&download_path, &bytes).await?;

        // Verify hash
        let actual_hash = Hash::hash_file(&download_path).await?;
        if actual_hash.to_hex() != expected_hash {
            fs::remove_file(&download_path).await?;
            return Err(BuildError::HashMismatch {
                file: filename.to_string(),
                expected: expected_hash.to_string(),
                actual: actual_hash.to_hex(),
            }
            .into());
        }

        self.downloads
            .insert(url.to_string(), download_path.clone());
        Ok(download_path)
    }

    /// Apply a patch file
    ///
    /// # Errors
    ///
    /// Returns an error if the patch command fails.
    pub async fn apply_patch(
        &self,
        patch_path: &Path,
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        env.execute_command(
            "patch",
            &["-p1", "-i", &patch_path.display().to_string()],
            Some(&self.working_dir),
        )
        .await
    }

    /// Configure with autotools
    ///
    /// # Errors
    ///
    /// Returns an error if the configure or make commands fail.
    pub async fn autotools(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{AutotoolsBuildSystem, BuildSystem, BuildSystemContext};

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        let autotools_system = AutotoolsBuildSystem::new();

        // Configure
        autotools_system.configure(&ctx, args).await?;

        // Build
        autotools_system.build(&ctx, &[]).await?;

        // Install - this will also adjust staged files
        autotools_system.install(&ctx).await?;

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Autotools build completed successfully".to_string(),
            stderr: String::new(),
        })
    }

    /// Configure with `CMake`
    ///
    /// # Errors
    ///
    /// Returns an error if the cmake or make commands fail.
    pub async fn cmake(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, CMakeBuildSystem};

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context with out-of-source build directory
        let build_dir = self.working_dir.join("build");
        fs::create_dir_all(&build_dir).await?;

        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.build_dir = build_dir;

        let cmake_system = CMakeBuildSystem::new();

        // Configure
        cmake_system.configure(&ctx, args).await?;

        // Build
        cmake_system.build(&ctx, &[]).await?;

        // Install - this will also adjust staged files
        cmake_system.install(&ctx).await?;

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "CMake build completed successfully".to_string(),
            stderr: String::new(),
        })
    }

    /// Configure with Meson
    ///
    /// # Errors
    ///
    /// Returns an error if the meson commands fail.
    pub async fn meson(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, MesonBuildSystem};

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context with out-of-source build directory
        let build_dir = self.working_dir.join("build");

        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.build_dir = build_dir;

        let meson_system = MesonBuildSystem::new();

        // Configure
        meson_system.configure(&ctx, args).await?;

        // Build
        meson_system.build(&ctx, &[]).await?;

        // Install - this will also adjust staged files
        meson_system.install(&ctx).await?;

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Meson build completed successfully".to_string(),
            stderr: String::new(),
        })
    }

    /// Build with Cargo
    ///
    /// # Errors
    ///
    /// Returns an error if the cargo command fails.
    ///
    /// # Panics
    ///
    /// Panics if the binary filename cannot be extracted from the path.
    pub async fn cargo(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, CargoBuildSystem};

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        let cargo_system = CargoBuildSystem::new();

        // Configure (checks Cargo.toml, sets up environment)
        cargo_system.configure(&ctx, args).await?;

        // Build
        cargo_system.build(&ctx, args).await?;

        // Install - this will copy binaries to staging/bin
        cargo_system.install(&ctx).await?;

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Cargo build completed successfully".to_string(),
            stderr: String::new(),
        })
    }

    /// Build with Go
    ///
    /// # Errors
    ///
    /// Returns an error if the go command fails.
    pub async fn go(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, GoBuildSystem};

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        let go_system = GoBuildSystem::new();

        // Configure if needed (this will handle go mod vendor, etc)
        go_system.configure(&ctx, args).await?;

        // Build the project - this will output to staging/bin automatically
        go_system.build(&ctx, args).await?;

        // Install (verifies binaries and sets permissions)
        go_system.install(&ctx).await?;

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Go build completed successfully".to_string(),
            stderr: String::new(),
        })
    }

    /// Build with Python
    ///
    /// # Errors
    ///
    /// Returns an error if the python3 command fails.
    pub async fn python(
        &mut self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, PythonBuildSystem};

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        let python_system = PythonBuildSystem::new();

        // Configure (detects build backend, sets up environment)
        python_system.configure(&ctx, args).await?;

        // Build (builds wheel or runs setup.py)
        python_system.build(&ctx, args).await?;

        // Install (installs to staging with BUILD_PREFIX)
        python_system.install(&ctx).await?;

        // Copy Python metadata from BuildSystemContext to BuilderApi
        if let Ok(extra_env) = ctx.extra_env.read() {
            for (key, value) in extra_env.iter() {
                if key.starts_with("PYTHON_") {
                    self.build_metadata.insert(key.clone(), value.clone());
                }
            }
        }

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Python build completed successfully".to_string(),
            stderr: String::new(),
        })
    }

    /// Build with Node.js
    ///
    /// # Errors
    ///
    /// Returns an error if the node/npm command fails.
    pub async fn nodejs(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, NodeJsBuildSystem};

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        let nodejs_system = NodeJsBuildSystem::new();

        // Configure (detects package manager, sets up environment)
        nodejs_system.configure(&ctx, args).await?;

        // Build (installs dependencies if needed, runs build scripts)
        nodejs_system.build(&ctx, args).await?;

        // Install (copies built artifacts and bin entries to staging)
        nodejs_system.install(&ctx).await?;

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Node.js build completed successfully".to_string(),
            stderr: String::new(),
        })
    }

    /// Run configure step only
    ///
    /// # Errors
    ///
    /// Returns an error if the configure command fails.
    pub async fn configure(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Add prefix if not already specified
        let mut configure_args = args.to_vec();
        if !configure_args
            .iter()
            .any(|arg| arg.starts_with("--prefix="))
        {
            configure_args.insert(0, "--prefix=/opt/pm/live".to_string());
        }

        env.execute_command(
            "sh",
            &["-c", &format!("./configure {}", configure_args.join(" "))],
            Some(&self.working_dir),
        )
        .await
    }

    /// Run make step only
    ///
    /// # Errors
    ///
    /// Returns an error if the make command fails.
    pub async fn make(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Process arguments, replacing relative DESTDIR with absolute path
        let processed_args: Vec<String> = args
            .iter()
            .map(|arg| {
                if arg.starts_with("DESTDIR=") {
                    // Always use the absolute staging directory from environment
                    format!("DESTDIR={}", env.staging_dir().display())
                } else {
                    arg.clone()
                }
            })
            .collect();

        let arg_strs: Vec<&str> = processed_args.iter().map(String::as_str).collect();
        env.execute_command("make", &arg_strs, Some(&self.working_dir))
            .await
    }

    /// Mark that installation is requested
    ///
    /// This method does not actually perform installation during recipe execution.
    /// Instead, it marks that the package should be installed after it's built.
    /// The actual installation happens after the .sp package is created.
    pub async fn install(&mut self, _env: &BuildEnvironment) -> Result<BuildCommandResult, Error> {
        // Mark that installation was requested
        self.install_requested = true;

        // Return success - the actual installation will happen later
        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Installation request recorded".to_string(),
            stderr: String::new(),
        })
    }

    /// Set SBOM generation
    #[must_use]
    pub fn auto_sbom(&mut self, enable: bool) -> &mut Self {
        self.auto_sbom = enable;
        self
    }

    /// Set SBOM exclusion patterns
    #[must_use]
    pub fn sbom_excludes(&mut self, patterns: Vec<String>) -> &mut Self {
        self.sbom_excludes = patterns;
        self
    }

    /// Get SBOM configuration
    #[must_use]
    pub fn sbom_config(&self) -> (bool, &[String]) {
        (self.auto_sbom, &self.sbom_excludes)
    }

    /// Check if installation was requested during recipe execution
    #[must_use]
    pub fn is_install_requested(&self) -> bool {
        self.install_requested
    }

    /// Get build metadata collected during build
    #[must_use]
    pub fn build_metadata(&self) -> &HashMap<String, String> {
        &self.build_metadata
    }

    /// Take build metadata (consumes the metadata)
    pub fn take_build_metadata(&mut self) -> HashMap<String, String> {
        std::mem::take(&mut self.build_metadata)
    }

    /// Extract downloaded archives
    ///
    /// # Errors
    ///
    /// Returns an error if any archive extraction fails.
    pub async fn extract_downloads(&self) -> Result<(), Error> {
        for path in self.downloads.values() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                match ext {
                    "gz" | "tgz" => {
                        self.extract_tar_gz(path).await?;
                    }
                    "bz2" => {
                        self.extract_tar_bz2(path).await?;
                    }
                    "xz" => {
                        self.extract_tar_xz(path).await?;
                    }
                    "zip" => {
                        self.extract_zip(path).await?;
                    }
                    _ => {
                        // Unknown format, skip extraction
                    }
                }
            }
        }
        Ok(())
    }

    /// Extract tar.gz archive
    ///
    /// # Errors
    ///
    /// Returns an error if the tar command fails.
    async fn extract_tar_gz(&self, path: &Path) -> Result<(), Error> {
        let output = tokio::process::Command::new("tar")
            .args(["-xzf", &path.display().to_string()])
            .current_dir(&self.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "Failed to extract {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Extract tar.bz2 archive
    ///
    /// # Errors
    ///
    /// Returns an error if the tar command fails.
    async fn extract_tar_bz2(&self, path: &Path) -> Result<(), Error> {
        let output = tokio::process::Command::new("tar")
            .args(["-xjf", &path.display().to_string()])
            .current_dir(&self.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "Failed to extract {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Extract tar.xz archive
    ///
    /// # Errors
    ///
    /// Returns an error if the tar command fails.
    async fn extract_tar_xz(&self, path: &Path) -> Result<(), Error> {
        let output = tokio::process::Command::new("tar")
            .args(["-xJf", &path.display().to_string()])
            .current_dir(&self.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "Failed to extract {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Extract zip archive
    ///
    /// # Errors
    ///
    /// Returns an error if the unzip command fails.
    async fn extract_zip(&self, path: &Path) -> Result<(), Error> {
        let output = tokio::process::Command::new("unzip")
            .args(["-q", &path.display().to_string()])
            .current_dir(&self.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "Failed to extract {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }
}

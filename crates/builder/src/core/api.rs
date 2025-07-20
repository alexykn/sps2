//! Builder API for Starlark recipes

use crate::environment::IsolationLevel;
use crate::{BuildCommandResult, BuildEnvironment};
use md5::{Digest, Md5};
use sha2::{Digest as Sha2Digest, Sha256};
use sps2_errors::{BuildError, Error};
use sps2_hash::Hash;
use sps2_net::{NetClient, NetConfig};
use sps2_types::RpathStyle;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

use sps2_resources::ResourceManager;
use std::sync::Arc;

/// Builder API exposed to Starlark recipes
#[derive(Clone)]
pub struct BuilderApi {
    /// Working directory for source extraction
    pub(crate) working_dir: PathBuf,
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
    /// Build isolation level (None if not explicitly set)
    explicit_isolation_level: Option<IsolationLevel>,
    /// Resource manager
    resources: Arc<ResourceManager>,
}

impl BuilderApi {
    /// Create new builder API
    ///
    /// # Errors
    ///
    /// Returns an error if the network client cannot be created.
    pub fn new(working_dir: PathBuf, resources: Arc<ResourceManager>) -> Result<Self, Error> {
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
            explicit_isolation_level: None,
            resources,
        })
    }

    /// Allow network access during build
    #[must_use]
    pub fn allow_network(&mut self, allow: bool) -> &mut Self {
        self.allow_network = allow;
        self
    }

    /// Update the working directory (used after git clone to point to the correct source)
    pub fn set_working_dir(&mut self, new_working_dir: PathBuf) {
        self.working_dir = new_working_dir;
    }

    /// Download a file
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Network access is disabled
    /// - The URL is invalid
    /// - The download fails
    pub async fn fetch(&mut self, url: &str) -> Result<PathBuf, Error> {
        // Fetch operations always have network access - they're source fetching, not build operations

        // Acquire a download permit
        let _permit = self.resources.acquire_download_permit().await?;

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

        // No hash verification - files are downloaded without validation

        self.downloads
            .insert(url.to_string(), download_path.clone());

        // Note: Extraction is handled separately by extract_downloads_to() method

        Ok(download_path)
    }

    /// Download and verify a file with MD5 hash
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL is invalid
    /// - The download fails
    /// - The file hash doesn't match the expected MD5 hash
    pub async fn fetch_md5(&mut self, url: &str, expected_md5: &str) -> Result<PathBuf, Error> {
        let download_path = self.fetch(url).await?;

        // Verify MD5 hash
        let bytes = tokio::fs::read(&download_path).await?;
        let mut hasher = Md5::new();
        hasher.update(&bytes);
        let actual_md5 = format!("{:x}", hasher.finalize());

        if actual_md5.to_lowercase() != expected_md5.to_lowercase() {
            tokio::fs::remove_file(&download_path).await?;
            let filename = download_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            return Err(BuildError::HashMismatch {
                file: filename.to_string(),
                expected: expected_md5.to_string(),
                actual: actual_md5,
            }
            .into());
        }

        Ok(download_path)
    }

    /// Download and verify a file with SHA256 hash
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL is invalid
    /// - The download fails
    /// - The file hash doesn't match the expected SHA256 hash
    pub async fn fetch_sha256(
        &mut self,
        url: &str,
        expected_sha256: &str,
    ) -> Result<PathBuf, Error> {
        let download_path = self.fetch(url).await?;

        // Verify SHA256 hash
        let bytes = tokio::fs::read(&download_path).await?;
        let mut hasher = Sha256::new();
        Sha2Digest::update(&mut hasher, &bytes);
        let actual_sha256 = format!("{:x}", hasher.finalize());

        if actual_sha256.to_lowercase() != expected_sha256.to_lowercase() {
            tokio::fs::remove_file(&download_path).await?;
            let filename = download_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            return Err(BuildError::HashMismatch {
                file: filename.to_string(),
                expected: expected_sha256.to_string(),
                actual: actual_sha256,
            }
            .into());
        }

        Ok(download_path)
    }

    /// Download and verify a file with BLAKE3 hash
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL is invalid
    /// - The download fails
    /// - The file hash doesn't match the expected BLAKE3 hash
    pub async fn fetch_blake3(
        &mut self,
        url: &str,
        expected_blake3: &str,
    ) -> Result<PathBuf, Error> {
        let download_path = self.fetch(url).await?;

        // Verify BLAKE3 hash specifically for download verification
        let actual_hash = Hash::blake3_hash_file(&download_path).await?;
        let actual_blake3 = actual_hash.to_hex();

        if actual_blake3.to_lowercase() != expected_blake3.to_lowercase() {
            tokio::fs::remove_file(&download_path).await?;
            let filename = download_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            return Err(BuildError::HashMismatch {
                file: filename.to_string(),
                expected: expected_blake3.to_string(),
                actual: actual_blake3,
            }
            .into());
        }

        Ok(download_path)
    }

    /// Clone a git repository
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Network access is disabled
    /// - The URL is invalid
    /// - The git clone fails
    pub async fn git(&mut self, url: &str, ref_: &str) -> Result<PathBuf, Error> {
        // Git operations always have network access - they're source fetching, not build operations

        // Check if already cloned
        if let Some(path) = self.downloads.get(url) {
            return Ok(path.clone());
        }

        // Extract repository name from URL
        let repo_name = url
            .split('/')
            .next_back()
            .and_then(|s| s.strip_suffix(".git").or(Some(s)))
            .ok_or_else(|| BuildError::InvalidUrl {
                url: url.to_string(),
            })?;

        let clone_path = self.working_dir.join(repo_name);

        // Clone using git command (better compatibility than git2 crate)
        let output = if ref_ == "HEAD" {
            // For HEAD, don't use --branch flag
            tokio::process::Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    url,
                    &clone_path.display().to_string(),
                ])
                .current_dir(&self.working_dir)
                .output()
                .await?
        } else {
            // For specific branches/tags, use --branch
            tokio::process::Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "--branch",
                    ref_,
                    url,
                    &clone_path.display().to_string(),
                ])
                .current_dir(&self.working_dir)
                .output()
                .await?
        };

        if !output.status.success() {
            return Err(BuildError::GitCloneFailed {
                message: format!(
                    "Failed to clone {}: {}",
                    url,
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        self.downloads.insert(url.to_string(), clone_path.clone());

        // Update working directory to the cloned path so subsequent operations
        // (like cargo build) work in the correct directory
        self.set_working_dir(clone_path.clone());

        Ok(clone_path)
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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{AutotoolsBuildSystem, BuildSystem, BuildSystemContext};

        // Record that we're using autotools build system
        env.record_build_system("autotools");

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.network_allowed = self.allow_network;
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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, CMakeBuildSystem};

        // Record that we're using cmake build system
        env.record_build_system("cmake");

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context with out-of-source build directory
        let build_dir = self.working_dir.join("build");
        fs::create_dir_all(&build_dir).await?;

        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.build_dir = build_dir;
        ctx.network_allowed = self.allow_network;

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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, MesonBuildSystem};

        // Record that we're using meson build system
        env.record_build_system("meson");

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context with out-of-source build directory
        let build_dir = self.working_dir.join("build");

        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.build_dir = build_dir;
        ctx.network_allowed = self.allow_network;

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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, CargoBuildSystem};

        // Record that we're using cargo build system
        env.record_build_system("cargo");

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.network_allowed = self.allow_network;
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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, GoBuildSystem};

        // Record that we're using go build system
        env.record_build_system("go");

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.network_allowed = self.allow_network;
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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, PythonBuildSystem};

        // Record that we're using python build system
        env.record_build_system("python");

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.network_allowed = self.allow_network;
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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::build_systems::{BuildSystem, BuildSystemContext, NodeJsBuildSystem};

        // Record that we're using nodejs build system
        env.record_build_system("nodejs");

        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build system context
        let mut ctx = BuildSystemContext::new(env.clone(), self.working_dir.clone());
        ctx.network_allowed = self.allow_network;
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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Record that we're using configure (part of autotools)
        env.record_build_system("configure");
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
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Record that we're using make (generic build system)
        env.record_build_system("make");
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
    ///
    /// # Errors
    ///
    /// This function currently never returns an error, but returns `Result` for API consistency
    pub fn install(&mut self, _env: &BuildEnvironment) -> Result<BuildCommandResult, Error> {
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

    /// Set isolation level
    pub fn set_isolation(&mut self, level: IsolationLevel) {
        self.explicit_isolation_level = Some(level);
    }

    /// Get isolation level if explicitly set
    #[must_use]
    pub fn explicit_isolation_level(&self) -> Option<IsolationLevel> {
        self.explicit_isolation_level
    }

    /// Check if isolation was explicitly set
    #[must_use]
    pub fn is_isolation_explicitly_set(&self) -> bool {
        self.explicit_isolation_level.is_some()
    }

    /// Extract downloaded archives
    ///
    /// # Errors
    ///
    /// Returns an error if any archive extraction fails.
    pub async fn extract_downloads(&self) -> Result<(), Error> {
        for path in self.downloads.values() {
            self.extract_single_download(path, None).await?;
        }
        Ok(())
    }

    /// Extract downloaded archives to a specific subdirectory
    ///
    /// # Errors
    ///
    /// Returns an error if any archive extraction fails.
    pub async fn extract_downloads_to(&self, extract_to: Option<&str>) -> Result<(), Error> {
        for path in self.downloads.values() {
            self.extract_single_download(path, extract_to).await?;
        }
        Ok(())
    }

    /// Extract a single downloaded file
    ///
    /// # Errors
    ///
    /// Returns an error if archive extraction fails.
    pub async fn extract_single_download(
        &self,
        path: &Path,
        extract_to: Option<&str>,
    ) -> Result<(), Error> {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext {
                "gz" | "tgz" => {
                    self.extract_tar_gz(path, extract_to).await?;
                }
                "bz2" => {
                    self.extract_tar_bz2(path, extract_to).await?;
                }
                "xz" => {
                    self.extract_tar_xz(path, extract_to).await?;
                }
                "zip" => {
                    self.extract_zip(path, extract_to).await?;
                }
                _ => {
                    // Unknown format, skip extraction
                }
            }
        } else {
            // For files without extensions (like GitHub API downloads), check magic numbers
            let file_bytes = tokio::fs::read(path).await.unwrap_or_default();
            if file_bytes.len() >= 4 {
                let magic = &file_bytes[0..4];
                // Check for gzip magic number (1f 8b)
                if magic[0] == 0x1f && magic[1] == 0x8b {
                    self.extract_tar_gz(path, extract_to).await?;
                }
                // Check for ZIP magic number (50 4b)
                else if magic[0] == 0x50 && magic[1] == 0x4b {
                    self.extract_zip(path, extract_to).await?;
                }
                // Check for bzip2 magic number (42 5a)
                else if magic[0] == 0x42 && magic[1] == 0x5a {
                    self.extract_tar_bz2(path, extract_to).await?;
                }
            }
        }
        Ok(())
    }

    /// Extract tar.gz archive
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails.
    async fn extract_tar_gz(&self, path: &Path, extract_to: Option<&str>) -> Result<(), Error> {
        self.extract_compressed_tar(path, CompressionType::Gzip, extract_to)
            .await
    }

    /// Extract tar.bz2 archive
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails.
    async fn extract_tar_bz2(&self, path: &Path, extract_to: Option<&str>) -> Result<(), Error> {
        self.extract_compressed_tar(path, CompressionType::Bzip2, extract_to)
            .await
    }

    /// Extract tar.xz archive
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails.
    async fn extract_tar_xz(&self, path: &Path, extract_to: Option<&str>) -> Result<(), Error> {
        self.extract_compressed_tar(path, CompressionType::Xz, extract_to)
            .await
    }

    /// Extract zip archive
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails.
    async fn extract_zip(&self, path: &Path, extract_to: Option<&str>) -> Result<(), Error> {
        let base_dir = if let Some(extract_to) = extract_to {
            // For multi-source builds, extract_to should be relative to the parent of working_dir
            if let Some(parent) = self.working_dir.parent() {
                parent.join(extract_to)
            } else {
                self.working_dir.join(extract_to)
            }
        } else {
            self.working_dir.clone()
        };
        let path_buf = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            use std::fs::File;
            use zip::ZipArchive;

            let file = File::open(&path_buf).map_err(|e| BuildError::ExtractionFailed {
                message: format!("Failed to open zip archive: {e}"),
            })?;

            let mut archive = ZipArchive::new(file).map_err(|e| BuildError::ExtractionFailed {
                message: format!("Failed to read zip archive: {e}"),
            })?;

            // Check if archive has a single top-level directory
            let strip_components = usize::from(should_strip_zip_components(&mut archive)?);

            for i in 0..archive.len() {
                let mut file = archive
                    .by_index(i)
                    .map_err(|e| BuildError::ExtractionFailed {
                        message: format!("Failed to read zip entry: {e}"),
                    })?;

                let outpath = match file.enclosed_name() {
                    Some(path) => {
                        // Strip components if needed
                        let components: Vec<_> = path.components().collect();
                        if strip_components > 0 && components.len() > strip_components {
                            base_dir
                                .join(components[strip_components..].iter().collect::<PathBuf>())
                        } else if strip_components == 0 {
                            base_dir.join(path)
                        } else {
                            continue; // Skip files at the stripped level
                        }
                    }
                    None => continue,
                };

                if file.name().ends_with('/') {
                    std::fs::create_dir_all(&outpath).map_err(|e| {
                        BuildError::ExtractionFailed {
                            message: format!("Failed to create directory: {e}"),
                        }
                    })?;
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            std::fs::create_dir_all(p).map_err(|e| {
                                BuildError::ExtractionFailed {
                                    message: format!("Failed to create parent directory: {e}"),
                                }
                            })?;
                        }
                    }
                    let mut outfile =
                        File::create(&outpath).map_err(|e| BuildError::ExtractionFailed {
                            message: format!("Failed to create file: {e}"),
                        })?;
                    std::io::copy(&mut file, &mut outfile).map_err(|e| {
                        BuildError::ExtractionFailed {
                            message: format!("Failed to extract file: {e}"),
                        }
                    })?;
                }

                // Set permissions on Unix
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Some(mode) = file.unix_mode() {
                        std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))
                            .ok();
                    }
                }
            }

            Ok::<(), Error>(())
        })
        .await
        .map_err(|e| BuildError::ExtractionFailed {
            message: format!("Task join error: {e}"),
        })?
    }

    /// Extract compressed tar archive using async-compression
    async fn extract_compressed_tar(
        &self,
        path: &Path,
        compression: CompressionType,
        extract_to: Option<&str>,
    ) -> Result<(), Error> {
        use async_compression::tokio::bufread::{BzDecoder, GzipDecoder, XzDecoder};
        use tokio::io::{AsyncWriteExt, BufReader};

        // Create a temporary file to decompress to
        let temp_dir = tempfile::tempdir().map_err(|e| BuildError::ExtractionFailed {
            message: format!("Failed to create temp directory: {e}"),
        })?;
        let temp_path = temp_dir.path().join("archive.tar");

        // Decompress the archive
        {
            use tokio::fs::File;

            let input_file = File::open(path)
                .await
                .map_err(|e| BuildError::ExtractionFailed {
                    message: format!("Failed to open archive: {e}"),
                })?;

            let mut output_file =
                File::create(&temp_path)
                    .await
                    .map_err(|e| BuildError::ExtractionFailed {
                        message: format!("Failed to create temp file: {e}"),
                    })?;
            let reader = BufReader::new(input_file);

            // Use specific decoders based on compression type
            match compression {
                CompressionType::Gzip => {
                    let mut decoder = GzipDecoder::new(reader);
                    tokio::io::copy(&mut decoder, &mut output_file)
                        .await
                        .map_err(|e| BuildError::ExtractionFailed {
                            message: format!("Failed to decompress gzip archive: {e}"),
                        })?;
                }
                CompressionType::Bzip2 => {
                    let mut decoder = BzDecoder::new(reader);
                    tokio::io::copy(&mut decoder, &mut output_file)
                        .await
                        .map_err(|e| BuildError::ExtractionFailed {
                            message: format!("Failed to decompress bzip2 archive: {e}"),
                        })?;
                }
                CompressionType::Xz => {
                    let mut decoder = XzDecoder::new(reader);
                    tokio::io::copy(&mut decoder, &mut output_file)
                        .await
                        .map_err(|e| BuildError::ExtractionFailed {
                            message: format!("Failed to decompress xz archive: {e}"),
                        })?;
                }
            }

            output_file
                .flush()
                .await
                .map_err(|e| BuildError::ExtractionFailed {
                    message: format!("Failed to flush temp file: {e}"),
                })?;
        }

        // Extract the decompressed tar file (keep temp_dir alive)
        let result = self.extract_tar_from_temp(&temp_path, extract_to).await;

        // temp_dir will be automatically cleaned up when it goes out of scope
        drop(temp_dir);

        result
    }

    /// Extract tar archive from temporary file
    async fn extract_tar_from_temp(
        &self,
        temp_path: &Path,
        extract_to: Option<&str>,
    ) -> Result<(), Error> {
        let base_dir = if let Some(extract_to) = extract_to {
            // For multi-source builds, extract_to should be relative to the parent of working_dir
            if let Some(parent) = self.working_dir.parent() {
                parent.join(extract_to)
            } else {
                self.working_dir.join(extract_to)
            }
        } else {
            self.working_dir.clone()
        };

        let temp_path_for_task = temp_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            use std::fs::File;
            use tar::Archive;

            let tar =
                File::open(&temp_path_for_task).map_err(|e| BuildError::ExtractionFailed {
                    message: format!("Failed to open decompressed file: {e}"),
                })?;
            let mut archive = Archive::new(tar);

            // Strip the first component (common for source archives)
            for entry in archive.entries()? {
                let mut entry = entry?;
                let path = entry.path()?;

                // Skip if path has no components or only one component
                let components: Vec<_> = path.components().collect();
                if components.len() <= 1 {
                    continue;
                }

                // Create new path without first component
                let new_path = components[1..].iter().collect::<PathBuf>();
                let dest_path = base_dir.join(&new_path);

                // Ensure parent directory exists
                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| BuildError::ExtractionFailed {
                        message: format!("Failed to create parent directory: {e}"),
                    })?;
                }

                entry
                    .unpack(&dest_path)
                    .map_err(|e| BuildError::ExtractionFailed {
                        message: format!("Failed to extract entry: {e}"),
                    })?;
            }

            Ok::<(), Error>(())
        })
        .await
        .map_err(|e| BuildError::ExtractionFailed {
            message: format!("Task join error: {e}"),
        })??;

        Ok(())
    }

    /// Copy source files from a directory to the working directory
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The source path doesn't exist
    /// - File copy operations fail
    /// - Directory creation fails
    pub async fn copy(
        &mut self,
        src_path: Option<&str>,
        context: &crate::BuildContext,
    ) -> Result<(), Error> {
        use crate::utils::fileops::copy_source_files;

        // Determine source path
        let source_dir = if let Some(path) = src_path {
            PathBuf::from(path)
        } else {
            // Use recipe directory if no path specified
            context
                .recipe_path
                .parent()
                .ok_or_else(|| BuildError::RecipeError {
                    message: "Invalid recipe path".to_string(),
                })?
                .to_path_buf()
        };

        // Check if source directory exists
        if !source_dir.exists() {
            return Err(BuildError::Failed {
                message: format!("Source directory does not exist: {}", source_dir.display()),
            }
            .into());
        }

        // Copy source files from the source directory to working directory
        copy_source_files(&source_dir, &self.working_dir, context).await?;

        Ok(())
    }

    /// Apply rpath patching to binaries and libraries
    ///
    /// # Errors
    ///
    /// Returns an error if the patching fails.
    pub async fn patch_rpaths(
        &self,
        style: RpathStyle,
        paths: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        use crate::artifact_qa::patchers::rpath::RPathPatcher;
        use sps2_types::BuildSystemProfile;

        // Check if this is appropriate for the build system
        let used_systems = env.used_build_systems();
        let profile = BuildSystemProfile::from_build_system(
            used_systems.iter().next().unwrap_or(&"unknown".to_string()),
        );

        if matches!(
            profile,
            BuildSystemProfile::RustMinimal | BuildSystemProfile::GoMedium
        ) {
            eprintln!(
                "Warning: patch_rpaths() is not recommended for {} projects. \
                Rust and Go binaries typically don't need rpath patching and it may cause issues.",
                if used_systems.contains("cargo") {
                    "Rust"
                } else {
                    "Go"
                }
            );
            // Continue but warn - don't fail in case user knows what they're doing
        }

        // Create a custom RPathPatcher with the specified style
        let patcher = RPathPatcher::new(style);

        // If specific paths are provided, patch only those
        // Otherwise, patch all binaries and libraries in staging
        let staging_dir = env.staging_dir();
        let lib_path = "/opt/pm/live/lib";
        let build_paths = vec!["/opt/pm/build".to_string(), "/private/".to_string()];

        let mut patched_count = 0;
        let mut errors = Vec::new();

        if paths.is_empty() {
            // Patch all eligible files
            use ignore::WalkBuilder;
            for entry in WalkBuilder::new(staging_dir)
                .hidden(false)
                .parents(false)
                .build()
                .filter_map(Result::ok)
            {
                let path = entry.into_path();
                if RPathPatcher::should_process_file(&path) {
                    let (_, was_fixed, _, error) =
                        patcher.process_file(&path, lib_path, &build_paths).await;
                    if was_fixed {
                        patched_count += 1;
                    }
                    if let Some(err) = error {
                        errors.push(err);
                    }
                }
            }
        } else {
            // Patch only specified paths
            for path_str in paths {
                let path = staging_dir.join(path_str);
                if path.exists() && RPathPatcher::should_process_file(&path) {
                    let (_, was_fixed, _, error) =
                        patcher.process_file(&path, lib_path, &build_paths).await;
                    if was_fixed {
                        patched_count += 1;
                    }
                    if let Some(err) = error {
                        errors.push(err);
                    }
                }
            }
        }

        let success = errors.is_empty();
        let stdout = format!("Patched {patched_count} files with {style} style");
        let stderr = if errors.is_empty() {
            String::new()
        } else {
            errors.join("\n")
        };

        Ok(BuildCommandResult {
            success,
            exit_code: Some(i32::from(!success)),
            stdout,
            stderr,
        })
    }

    /// Fix executable permissions on binaries
    ///
    /// # Errors
    ///
    /// Returns an error if permission fixing fails.
    pub fn fix_permissions(
        &self,
        paths: &[String],
        env: &mut BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Just record the request - actual fixing happens at the very end
        env.record_fix_permissions_request(paths.to_vec());

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Permissions fix scheduled for final packaging step".to_string(),
            stderr: String::new(),
        })
    }

    /// Internal implementation that actually fixes permissions
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to access files in the staging directory
    /// - Failed to modify file permissions
    /// - I/O errors occur during permission changes
    pub fn do_fix_permissions(
        &self,
        paths: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        let staging_dir = env.staging_dir();
        let mut fixed_count = 0;
        let mut errors = Vec::new();

        // Log initial state
        Self::log_fix_permissions_start(paths, staging_dir, env);

        // Process files based on whether specific paths were provided
        if paths.is_empty() {
            Self::fix_all_files(staging_dir, env, &mut fixed_count, &mut errors);
        } else {
            Self::fix_specific_paths(paths, staging_dir, &mut fixed_count, &mut errors);
        }

        // Build and return result
        Ok(Self::build_fix_permissions_result(
            fixed_count,
            &errors,
            staging_dir,
            env,
        ))
    }

    /// Log the start of `fix_permissions` operation
    fn log_fix_permissions_start(paths: &[String], staging_dir: &Path, env: &BuildEnvironment) {
        let dir_exists = staging_dir.exists();
        let is_dir = staging_dir.is_dir();

        crate::utils::events::send_event(
            &env.context,
            sps2_events::Event::debug(format!(
                "fix_permissions called with {} paths, staging_dir: {} (exists: {}, is_dir: {})",
                if paths.is_empty() {
                    "no specific".to_string()
                } else {
                    paths.len().to_string()
                },
                staging_dir.display(),
                dir_exists,
                is_dir
            )),
        );
    }

    /// Fix permissions for specific paths
    fn fix_specific_paths(
        paths: &[String],
        staging_dir: &Path,
        fixed_count: &mut usize,
        errors: &mut Vec<String>,
    ) {
        use crate::artifact_qa::patchers::permissions::PermissionsFixer;

        for path_str in paths {
            let full_path = staging_dir.join(path_str);
            if full_path.is_file()
                && PermissionsFixer::needs_execute_permission_aggressive(&full_path)
            {
                match fix_file_permissions(&full_path) {
                    Ok(true) => *fixed_count += 1,
                    Ok(false) => {} // Already had permissions
                    Err(e) => {
                        errors.push(format!("Failed to fix {}: {}", full_path.display(), e));
                    }
                }
            }
        }
    }

    /// Fix permissions for all files in staging directory
    fn fix_all_files(
        staging_dir: &Path,
        env: &BuildEnvironment,
        fixed_count: &mut usize,
        errors: &mut Vec<String>,
    ) {
        use crate::artifact_qa::patchers::permissions::PermissionsFixer;
        use ignore::WalkBuilder;

        let mut total_files = 0;
        let mut checked_files = 0;

        crate::utils::events::send_event(
            &env.context,
            sps2_events::Event::debug(format!(
                "Walking directory tree from: {}",
                staging_dir.display()
            )),
        );

        for entry in WalkBuilder::new(staging_dir)
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if path.is_file() {
                total_files += 1;

                // Debug logging for first few files
                if total_files <= 3
                    || (path.to_string_lossy().contains("/bin/") && total_files <= 10)
                {
                    let needs_fix = PermissionsFixer::needs_execute_permission_aggressive(&path);
                    crate::utils::events::send_event(
                        &env.context,
                        sps2_events::Event::debug(format!(
                            "File #{}: {} - needs fix: {}",
                            total_files,
                            path.display(),
                            needs_fix
                        )),
                    );
                }

                if PermissionsFixer::needs_execute_permission_aggressive(&path) {
                    checked_files += 1;
                    match fix_file_permissions(&path) {
                        Ok(true) => *fixed_count += 1,
                        Ok(false) => {} // Already had permissions
                        Err(e) => {
                            errors.push(format!("Failed to fix {}: {}", path.display(), e));
                        }
                    }
                }
            }
        }

        crate::utils::events::send_event(
            &env.context,
            sps2_events::Event::debug(format!(
                "fix_permissions: Checked {}/{} files, fixed {}",
                checked_files, total_files, *fixed_count
            )),
        );
    }

    /// Build the result for `fix_permissions` operation
    fn build_fix_permissions_result(
        fixed_count: usize,
        errors: &[String],
        staging_dir: &Path,
        env: &BuildEnvironment,
    ) -> BuildCommandResult {
        let success = errors.is_empty();
        let stdout = if fixed_count > 0 {
            format!("Fixed permissions on {fixed_count} files")
        } else {
            "No files needed permission fixes".to_string()
        };

        let stderr = if errors.is_empty() {
            String::new()
        } else {
            errors.join("\n")
        };

        // Log the result
        if fixed_count > 0 {
            crate::utils::events::send_event(
                &env.context,
                sps2_events::Event::debug(format!("fix_permissions: {stdout}")),
            );
        } else {
            crate::utils::events::send_event(
                &env.context,
                sps2_events::Event::debug(format!(
                    "fix_permissions: No files needed fixes in {}",
                    staging_dir.display()
                )),
            );
        }

        BuildCommandResult {
            success,
            exit_code: Some(i32::from(!success)),
            stdout,
            stderr,
        }
    }
}

/// Fix permissions on a file if needed
fn fix_file_permissions(path: &std::path::Path) -> Result<bool, std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)?;
    let mut perms = metadata.permissions();
    let current_mode = perms.mode();

    // Check if any execute bit is already set
    if current_mode & 0o111 != 0 {
        return Ok(false); // Already has execute permissions
    }

    // Add execute permissions matching read permissions
    // If readable by owner, make executable by owner, etc.
    let new_mode = current_mode | ((current_mode & 0o444) >> 2); // Convert read bits to execute bits

    perms.set_mode(new_mode);
    std::fs::set_permissions(path, perms)?;

    Ok(true)
}

/// Compression types
enum CompressionType {
    Gzip,
    Bzip2,
    Xz,
}

/// Check if a zip archive should have its first component stripped
fn should_strip_zip_components(
    archive: &mut zip::ZipArchive<std::fs::File>,
) -> Result<bool, Error> {
    let mut top_level_dirs = std::collections::HashSet::new();
    let mut has_files_at_root = false;

    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| BuildError::ExtractionFailed {
                message: format!("Failed to read zip entry: {e}"),
            })?;

        if let Some(path) = file.enclosed_name() {
            let components: Vec<_> = path.components().collect();
            if components.is_empty() {
                continue;
            }

            if components.len() == 1 {
                if file.name().ends_with('/') {
                    top_level_dirs.insert(components[0].as_os_str().to_string_lossy().to_string());
                } else {
                    has_files_at_root = true;
                }
            }
        }
    }

    // Strip if there's exactly one directory at top level and no files
    Ok(top_level_dirs.len() == 1 && !has_files_at_root)
}

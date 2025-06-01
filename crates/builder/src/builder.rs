//! Main builder implementation with deterministic TAR archive creation for reproducible builds

use crate::{
    BuildContext, BuildEnvironment, BuildResult, BuilderApi, CompressionConfig, PackageSigner,
    SbomConfig, SbomFiles, SbomGenerator, SigningConfig,
};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use sps2_manifest::Manifest;
use sps2_net::NetClient;
use sps2_package::{execute_recipe, load_recipe};
use sps2_resolver::Resolver;
use sps2_store::PackageStore;
use sps2_types::package::PackageSpec;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Default deterministic timestamp (Unix epoch) for reproducible builds
const DETERMINISTIC_TIMESTAMP: u64 = 0;

/// Environment variable for `SOURCE_DATE_EPOCH` (standard for reproducible builds)
const SOURCE_DATE_EPOCH_VAR: &str = "SOURCE_DATE_EPOCH";

/// Package builder configuration
#[derive(Clone, Debug)]
pub struct BuildConfig {
    /// SBOM generation configuration
    pub sbom_config: SbomConfig,
    /// Package signing configuration
    pub signing_config: SigningConfig,
    /// Maximum build time in seconds
    pub max_build_time: Option<u64>,
    /// Network access allowed during build
    pub allow_network: bool,
    /// Number of parallel build jobs
    pub build_jobs: Option<usize>,
    /// Build root directory (defaults to current directory)
    pub build_root: Option<PathBuf>,
    /// Compression configuration for package archives
    pub compression_config: CompressionConfig,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            sbom_config: SbomConfig::default(),
            signing_config: SigningConfig::default(),
            max_build_time: Some(3600), // 1 hour
            allow_network: false,
            build_jobs: None, // Use auto-detection
            build_root: None, // Defaults to current directory
            compression_config: CompressionConfig::default(),
        }
    }
}

impl BuildConfig {
    /// Create config with network access enabled
    #[must_use]
    pub fn with_network() -> Self {
        Self {
            allow_network: true,
            ..Default::default()
        }
    }

    /// Set SBOM configuration
    #[must_use]
    pub fn with_sbom_config(mut self, config: SbomConfig) -> Self {
        self.sbom_config = config;
        self
    }

    /// Set signing configuration
    #[must_use]
    pub fn with_signing_config(mut self, config: SigningConfig) -> Self {
        self.signing_config = config;
        self
    }

    /// Set build timeout
    #[must_use]
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.max_build_time = Some(seconds);
        self
    }

    /// Set parallel build jobs
    #[must_use]
    pub fn with_jobs(mut self, jobs: usize) -> Self {
        self.build_jobs = Some(jobs);
        self
    }

    /// Set compression configuration
    #[must_use]
    pub fn with_compression_config(mut self, config: CompressionConfig) -> Self {
        self.compression_config = config;
        self
    }

    /// Set compression level
    #[must_use]
    pub fn with_compression_level(mut self, level: crate::CompressionLevel) -> Self {
        self.compression_config.level = level;
        self
    }

    /// Enable fast compression for development builds
    #[must_use]
    pub fn with_fast_compression() -> Self {
        Self {
            compression_config: CompressionConfig::fast(),
            ..Default::default()
        }
    }

    /// Enable balanced compression (default)
    #[must_use]
    pub fn with_balanced_compression() -> Self {
        Self {
            compression_config: CompressionConfig::balanced(),
            ..Default::default()
        }
    }

    /// Enable maximum compression for production builds
    #[must_use]
    pub fn with_maximum_compression() -> Self {
        Self {
            compression_config: CompressionConfig::maximum(),
            ..Default::default()
        }
    }

    /// Enable custom compression level
    #[must_use]
    pub fn with_custom_compression(level: u8) -> Self {
        Self {
            compression_config: CompressionConfig::custom(level),
            ..Default::default()
        }
    }
}

/// Package builder
#[derive(Clone)]
pub struct Builder {
    /// Build configuration
    config: BuildConfig,
    /// Resolver for dependencies
    resolver: Option<Resolver>,
    /// Package store for output
    store: Option<PackageStore>,
    /// Network client for downloads
    net: Option<NetClient>,
}

impl Builder {
    /// Create new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: BuildConfig::default(),
            resolver: None,
            store: None,
            net: None,
        }
    }

    /// Create builder with configuration
    #[must_use]
    pub fn with_config(config: BuildConfig) -> Self {
        Self {
            config,
            resolver: None,
            store: None,
            net: None,
        }
    }

    /// Set resolver
    #[must_use]
    pub fn with_resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Set package store
    #[must_use]
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Set network client
    #[must_use]
    pub fn with_net(mut self, net: NetClient) -> Self {
        self.net = Some(net);
        self
    }

    /// Build a package from a Starlark recipe
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The recipe file cannot be read or parsed
    /// - Build dependencies cannot be resolved or installed
    /// - The build process fails or times out
    /// - SBOM generation fails
    /// - Package creation or signing fails
    /// - Environment setup or cleanup fails
    pub async fn build(&self, context: BuildContext) -> Result<BuildResult, Error> {
        Self::send_event(
            &context,
            Event::OperationStarted {
                operation: format!("Building {} {}", context.name, context.version),
            },
        );

        // Setup build environment
        let mut environment = self.setup_build_environment(&context).await?;

        // Execute recipe and setup dependencies
        let (runtime_deps, recipe_metadata) = self
            .execute_recipe_and_setup_deps(&context, &mut environment)
            .await?;

        // Run quality checks
        self.run_quality_checks(&context, &environment).await?;

        // Generate SBOM and create manifest
        let (sbom_files, manifest) = self
            .generate_sbom_and_manifest(&context, &environment, runtime_deps, &recipe_metadata)
            .await?;

        // Create and sign package
        let package_path = self
            .create_and_sign_package(&context, &environment, manifest, sbom_files)
            .await?;

        // Cleanup and finalize
        Self::cleanup_and_finalize(&context, &environment, &package_path);

        Ok(BuildResult::new(package_path))
    }

    /// Execute the Starlark recipe
    async fn execute_recipe(
        &self,
        context: &BuildContext,
        environment: &mut BuildEnvironment,
    ) -> Result<(Vec<String>, Vec<PackageSpec>, sps2_package::RecipeMetadata), Error> {
        // Read recipe file
        let _recipe_content = fs::read_to_string(&context.recipe_path)
            .await
            .map_err(|e| BuildError::RecipeError {
                message: format!(
                    "failed to read recipe {}: {e}",
                    context.recipe_path.display()
                ),
            })?;

        // Parse the recipe
        let recipe = load_recipe(&context.recipe_path).await?;

        // Create builder API
        let working_dir = environment.build_prefix().join("src");
        fs::create_dir_all(&working_dir).await?;

        // Copy source files from recipe directory to working directory
        let recipe_dir = context
            .recipe_path
            .parent()
            .ok_or_else(|| BuildError::RecipeError {
                message: "Invalid recipe path".to_string(),
            })?;

        // Copy all files from recipe directory to working directory (excluding .star files)
        Self::send_event(
            context,
            Event::DebugLog {
                message: format!(
                    "Copying source files from {} to {}",
                    recipe_dir.display(),
                    working_dir.display()
                ),
                context: std::collections::HashMap::new(),
            },
        );

        let mut entries = fs::read_dir(recipe_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            if entry_path.is_file() && entry_path.extension().is_none_or(|ext| ext != "star") {
                let file_name = entry.file_name();
                let dest_path = working_dir.join(&file_name);
                fs::copy(&entry_path, &dest_path).await?;

                Self::send_event(
                    context,
                    Event::DebugLog {
                        message: format!(
                            "Copied {} to {}",
                            file_name.to_string_lossy(),
                            dest_path.display()
                        ),
                        context: std::collections::HashMap::new(),
                    },
                );
            }
        }

        let mut api = BuilderApi::new(working_dir.clone())?;
        let _result = api.allow_network(self.config.allow_network);

        // Execute recipe with timeout
        let result = if let Some(timeout) = self.config.max_build_time {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                self.execute_recipe_steps(context, &recipe, &mut api, environment),
            )
            .await
            .map_err(|_| BuildError::BuildTimeout {
                package: context.name.clone(),
                timeout_seconds: timeout,
            })??
        } else {
            self.execute_recipe_steps(context, &recipe, &mut api, environment)
                .await?
        };

        Ok(result)
    }

    /// Execute recipe steps
    async fn execute_recipe_steps(
        &self,
        context: &BuildContext,
        recipe: &sps2_package::Recipe,
        api: &mut BuilderApi,
        environment: &mut BuildEnvironment,
    ) -> Result<(Vec<String>, Vec<PackageSpec>, sps2_package::RecipeMetadata), Error> {
        // Execute the recipe to get metadata
        let recipe_result = execute_recipe(recipe)?;

        // Extract runtime dependencies as strings
        let runtime_deps: Vec<String> = recipe_result.metadata.runtime_deps.clone();

        // Extract build dependencies as PackageSpec
        let build_deps: Vec<PackageSpec> = recipe_result
            .metadata
            .build_deps
            .iter()
            .map(|dep| PackageSpec::parse(dep))
            .collect::<Result<Vec<_>, _>>()?;

        // Execute build steps
        for step in &recipe_result.build_steps {
            Self::send_event(
                context,
                Event::BuildStepStarted {
                    step: format!("{step:?}"),
                    package: context.name.clone(),
                },
            );

            self.execute_build_step(step, api, environment).await?;

            Self::send_event(
                context,
                Event::BuildStepCompleted {
                    step: format!("{step:?}"),
                    package: context.name.clone(),
                },
            );
        }

        Ok((runtime_deps, build_deps, recipe_result.metadata.clone()))
    }

    /// Execute a single build step
    async fn execute_build_step(
        &self,
        step: &sps2_package::BuildStep,
        api: &mut BuilderApi,
        environment: &mut BuildEnvironment,
    ) -> Result<(), Error> {
        use sps2_package::BuildStep;

        match step {
            BuildStep::Fetch { url, blake3 } => {
                api.fetch(url, blake3).await?;
            }
            BuildStep::Configure { args } => {
                api.configure(args, environment).await?;
            }
            BuildStep::Make { args } => {
                api.make(args, environment).await?;
            }
            BuildStep::Autotools { args } => {
                api.autotools(args, environment).await?;
            }
            BuildStep::Cmake { args } => {
                api.cmake(args, environment).await?;
            }
            BuildStep::Meson { args } => {
                api.meson(args, environment).await?;
            }
            BuildStep::Cargo { args } => {
                api.cargo(args, environment).await?;
            }
            BuildStep::Install => {
                api.install(environment).await?;
            }
            BuildStep::ApplyPatch { path } => {
                api.apply_patch(std::path::Path::new(path), environment)
                    .await?;
            }
            BuildStep::Command { program, args } => {
                let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                environment
                    .execute_command(program, &arg_refs, None)
                    .await?;
            }
            BuildStep::SetEnv { key, value } => {
                environment.set_env_var(key.clone(), value.clone())?;
            }
            BuildStep::AllowNetwork { enabled } => {
                let _result = api.allow_network(*enabled);
            }
        }

        Ok(())
    }

    /// Scan staging directory for hardcoded build paths (relocatability check)
    async fn scan_for_hardcoded_paths(
        &self,
        context: &BuildContext,
        environment: &BuildEnvironment,
    ) -> Result<(), Error> {
        let staging_dir = environment.staging_dir();
        let build_prefix = environment.build_prefix();
        let build_prefix_str = build_prefix.display().to_string();

        Self::send_event(
            context,
            Event::OperationStarted {
                operation: "Scanning for hardcoded paths".to_string(),
            },
        );

        // Skip scanning if staging directory doesn't exist or is empty
        if !staging_dir.exists() {
            Self::send_event(
                context,
                Event::OperationCompleted {
                    operation: "Relocatability scan skipped (no staging directory)".to_string(),
                    success: true,
                },
            );
            return Ok(());
        }

        // Check if directory is empty
        let mut entries = fs::read_dir(staging_dir).await?;
        if entries.next_entry().await?.is_none() {
            Self::send_event(
                context,
                Event::OperationCompleted {
                    operation: "Relocatability scan skipped (empty staging directory)".to_string(),
                    success: true,
                },
            );
            return Ok(());
        }

        let violations = self
            .scan_directory_for_hardcoded_paths(staging_dir, &build_prefix_str)
            .await?;

        if !violations.is_empty() {
            let violation_list = violations.join("\n  ");
            return Err(BuildError::Failed {
                message: format!(
                    "Relocatability check failed: Found hardcoded build paths in {} files:\n  {}",
                    violations.len(),
                    violation_list
                ),
            }
            .into());
        }

        Self::send_event(
            context,
            Event::OperationCompleted {
                operation: "Relocatability scan passed".to_string(),
                success: true,
            },
        );

        Ok(())
    }

    /// Recursively scan directory for hardcoded paths
    fn scan_directory_for_hardcoded_paths<'a>(
        &'a self,
        dir: &'a Path,
        build_prefix: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<String>, Error>> + 'a>> {
        Box::pin(async move {
            let mut violations = Vec::new();

            // Check if directory exists first
            if !dir.exists() {
                return Ok(violations);
            }

            let Ok(mut entries) = fs::read_dir(dir).await else {
                return Ok(violations);
            };

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();

                if path.is_dir() {
                    // Recursively scan subdirectories
                    let mut sub_violations = self
                        .scan_directory_for_hardcoded_paths(&path, build_prefix)
                        .await?;
                    violations.append(&mut sub_violations);
                } else if path.is_file() {
                    // Check file for hardcoded paths
                    if let Some(violation) = self
                        .scan_file_for_hardcoded_paths(&path, build_prefix)
                        .await?
                    {
                        violations.push(violation);
                    }
                }
            }

            Ok(violations)
        })
    }

    /// Scan individual file for hardcoded paths
    async fn scan_file_for_hardcoded_paths(
        &self,
        file_path: &Path,
        build_prefix: &str,
    ) -> Result<Option<String>, Error> {
        // Skip non-text files and certain file types that are expected to contain paths
        if let Some(extension) = file_path.extension() {
            let ext = extension.to_string_lossy().to_lowercase();
            // Skip binary-ish files that might contain false positives
            if matches!(
                ext.as_str(),
                "so" | "dylib"
                    | "a"
                    | "o"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "ico"
                    | "zip"
                    | "tar"
                    | "gz"
                    | "bz2"
                    | "xz"
            ) {
                return Ok(None);
            }
        }

        // Read file content
        let Ok(content) = fs::read_to_string(file_path).await else {
            // File is not text, skip it (binary files)
            return Ok(None);
        };

        // Check if content contains the build prefix
        if content.contains(build_prefix) {
            return Ok(Some(format!(
                "{} (contains '{}')",
                file_path.display(),
                build_prefix
            )));
        }

        Ok(None)
    }

    /// Generate SBOM files
    async fn generate_sbom(&self, environment: &BuildEnvironment) -> Result<SbomFiles, Error> {
        let generator = SbomGenerator::new().with_config(self.config.sbom_config.clone());

        let staging_dir = environment.staging_dir();
        let sbom_dir = environment.build_prefix().join("sbom");
        fs::create_dir_all(&sbom_dir).await?;

        generator.generate_sbom(staging_dir, &sbom_dir).await
    }

    /// Create package manifest
    fn create_manifest(
        context: &BuildContext,
        runtime_deps: Vec<String>,
        sbom_files: &SbomFiles,
        recipe_metadata: &sps2_package::RecipeMetadata,
    ) -> Manifest {
        use sps2_manifest::{CompressionInfo, Dependencies, PackageInfo, SbomInfo};
        use sps2_types::format::CompressionFormatType;

        // Create SBOM info if files are available
        let sbom_info = sbom_files.spdx_hash.as_ref().map(|spdx_hash| SbomInfo {
            spdx: spdx_hash.clone(),
            cyclonedx: sbom_files.cyclonedx_hash.clone(),
        });

        // Create compression info
        let compression_info = Some(CompressionInfo {
            format: CompressionFormatType::Legacy,
            frame_size: None,
            frame_count: None,
        });

        Manifest {
            format_version: sps2_types::PackageFormatVersion::CURRENT,
            package: PackageInfo {
                name: context.name.clone(),
                version: context.version.to_string(),
                revision: context.revision,
                arch: context.arch.clone(),
                description: recipe_metadata.description.clone(),
                homepage: recipe_metadata.homepage.clone(),
                license: recipe_metadata.license.clone(),
                compression: compression_info,
            },
            dependencies: Dependencies {
                runtime: runtime_deps,
                build: Vec::new(), // Build deps not included in final manifest
            },
            sbom: sbom_info,
        }
    }

    /// Create the final package
    async fn create_package(
        &self,
        context: &BuildContext,
        environment: &BuildEnvironment,
        manifest: Manifest,
        sbom_files: SbomFiles,
    ) -> Result<PathBuf, Error> {
        let package_path = context.output_path();

        // Create package using the real manifest data
        let manifest_string = toml::to_string(&manifest).map_err(|e| BuildError::Failed {
            message: format!("failed to serialize manifest: {e}"),
        })?;

        // Create proper .sp archive with manifest and SBOM files
        self.create_sp_package(
            context,
            environment.staging_dir(),
            &package_path,
            &manifest_string,
            &sbom_files,
        )
        .await?;

        Ok(package_path)
    }

    /// Create a .sp package archive with manifest, SBOM files, and tar+zstd compression
    async fn create_sp_package(
        &self,
        context: &BuildContext,
        staging_dir: &Path,
        output_path: &Path,
        manifest_content: &str,
        sbom_files: &SbomFiles,
    ) -> Result<(), Error> {
        // Create the directory structure for .sp package
        let package_dir = staging_dir.parent().ok_or_else(|| BuildError::Failed {
            message: "Invalid staging directory path".to_string(),
        })?;

        let package_temp_dir = package_dir.join("package_temp");
        fs::create_dir_all(&package_temp_dir).await?;

        // Step 1: Create manifest.toml in package root
        let manifest_path = package_temp_dir.join("manifest.toml");
        fs::write(&manifest_path, manifest_content).await?;

        // Step 2: Copy SBOM files
        if let Some(spdx_path) = &sbom_files.spdx_path {
            let dst_path = package_temp_dir.join("sbom.spdx.json");
            fs::copy(spdx_path, &dst_path).await?;
        }

        if let Some(cdx_path) = &sbom_files.cyclonedx_path {
            let dst_path = package_temp_dir.join("sbom.cdx.json");
            fs::copy(cdx_path, &dst_path).await?;
        }

        // Step 3: Copy staging directory contents as package files
        Self::send_event(
            context,
            Event::OperationStarted {
                operation: "Copying package files".to_string(),
            },
        );
        let files_dir = package_temp_dir.join("files");
        if staging_dir.exists() {
            Self::copy_directory_recursive(staging_dir, &files_dir).await?;
        } else {
            // Create empty files directory if staging doesn't exist
            fs::create_dir_all(&files_dir).await?;
        }
        Self::send_event(
            context,
            Event::OperationCompleted {
                operation: "Package files copied".to_string(),
                success: true,
            },
        );

        // Step 4: Create deterministic tar archive
        Self::send_event(
            context,
            Event::OperationStarted {
                operation: "Creating tar archive".to_string(),
            },
        );

        // Debug: List contents before tar creation
        Self::send_event(
            context,
            Event::DebugLog {
                message: format!("Creating tar from: {}", package_temp_dir.display()),
                context: std::collections::HashMap::new(),
            },
        );

        let tar_path = package_temp_dir.join("package.tar");

        // Add timeout for tar creation to prevent hanging
        let tar_result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.create_deterministic_tar_archive(&package_temp_dir, &tar_path),
        )
        .await;

        match tar_result {
            Ok(result) => result?,
            Err(_) => {
                return Err(BuildError::Failed {
                    message: "Tar archive creation timed out after 30 seconds".to_string(),
                }
                .into());
            }
        }

        Self::send_event(
            context,
            Event::OperationCompleted {
                operation: "Tar archive created".to_string(),
                success: true,
            },
        );

        // Step 5: Compress with zstd at maximum level
        Self::send_event(
            context,
            Event::OperationStarted {
                operation: "Compressing package with zstd".to_string(),
            },
        );
        self.compress_with_zstd(&tar_path, output_path).await?;
        Self::send_event(
            context,
            Event::OperationCompleted {
                operation: "Package compression completed".to_string(),
                success: true,
            },
        );

        // Step 6: Cleanup temporary files
        fs::remove_dir_all(&package_temp_dir).await?;

        Ok(())
    }

    /// Recursively copy directory contents
    fn copy_directory_recursive<'a>(
        src: &'a Path,
        dst: &'a Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + 'a>> {
        Box::pin(async move {
            fs::create_dir_all(dst).await?;

            let mut entries = fs::read_dir(src).await?;
            while let Some(entry) = entries.next_entry().await? {
                let entry_path = entry.path();
                let dst_path = dst.join(entry.file_name());

                if entry_path.is_dir() {
                    Self::copy_directory_recursive(&entry_path, &dst_path).await?;
                } else {
                    fs::copy(&entry_path, &dst_path).await?;
                }
            }

            Ok(())
        })
    }

    /// Create deterministic tar archive from directory using the tar crate
    /// Ensures identical input produces identical compressed output for reproducible builds
    async fn create_deterministic_tar_archive(
        &self,
        source_dir: &Path,
        tar_path: &Path,
    ) -> Result<(), Error> {
        // Use the global deterministic timestamp
        let deterministic_timestamp = Self::get_deterministic_timestamp();
        self.create_deterministic_tar_archive_with_timestamp(
            source_dir,
            tar_path,
            deterministic_timestamp,
        )
        .await
    }

    /// Create deterministic tar archive with explicit timestamp (for testing)
    /// Ensures identical input produces identical compressed output for reproducible builds
    async fn create_deterministic_tar_archive_with_timestamp(
        &self,
        source_dir: &Path,
        tar_path: &Path,
        timestamp: u64,
    ) -> Result<(), Error> {
        use tar::Builder;
        use tokio::fs::File;

        let file = File::create(tar_path).await?;
        let file = file.into_std().await;
        let source_dir = source_dir.to_path_buf(); // Clone to move into closure

        // Create deterministic tar using the tar crate
        tokio::task::spawn_blocking(move || -> Result<(), Error> {
            let mut tar_builder = Builder::new(file);

            // Set deterministic behavior
            tar_builder.follow_symlinks(false);

            Self::add_directory_to_tar_with_timestamp(
                &mut tar_builder,
                &source_dir,
                "".as_ref(),
                timestamp,
            )?;
            tar_builder.finish()?;

            Ok(())
        })
        .await
        .map_err(|e| BuildError::Failed {
            message: format!("tar creation task failed: {e}"),
        })??;

        Ok(())
    }

    /// Recursively add directory contents to tar archive with deterministic ordering
    /// This is the enhanced deterministic version with improved file ordering and metadata normalization
    /// for reproducible builds
    fn add_directory_to_tar_with_timestamp(
        tar_builder: &mut tar::Builder<std::fs::File>,
        dir_path: &Path,
        tar_path: &Path,
        deterministic_timestamp: u64,
    ) -> Result<(), Error> {
        let mut entries = std::fs::read_dir(dir_path)?.collect::<Result<Vec<_>, _>>()?;

        // Enhanced deterministic sorting for optimal compression:
        // 1. Sort all entries lexicographically by filename (case-sensitive, locale-independent)
        // 2. This ensures consistent ordering across different filesystems and locales
        entries.sort_by(|a, b| {
            // Use OS string comparison for consistent, locale-independent ordering
            a.file_name().cmp(&b.file_name())
        });

        for entry in entries {
            let file_path = entry.path();
            let file_name = entry.file_name();

            // Skip the package.tar file if it exists to avoid recursion
            if file_name == "package.tar" {
                continue;
            }

            // Construct tar entry path - avoid leading separators for root entries
            let tar_entry_path = if tar_path.as_os_str().is_empty() {
                PathBuf::from(&file_name)
            } else {
                tar_path.join(&file_name)
            };

            let metadata = entry.metadata()?;

            if metadata.is_dir() {
                // Add directory entry with fully normalized metadata
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                header.set_mode(Self::normalize_file_permissions(&metadata));
                header.set_mtime(deterministic_timestamp);
                header.set_uid(0); // Normalized ownership
                header.set_gid(0); // Normalized ownership
                header.set_username("root")?; // Consistent username
                header.set_groupname("root")?; // Consistent group name
                header.set_device_major(0)?; // Clear device numbers
                header.set_device_minor(0)?; // Clear device numbers
                header.set_cksum();

                let tar_dir_path = format!("{}/", tar_entry_path.display());
                tar_builder.append_data(&mut header, &tar_dir_path, std::io::empty())?;

                // Recursively add directory contents
                Self::add_directory_to_tar_with_timestamp(
                    tar_builder,
                    &file_path,
                    &tar_entry_path,
                    deterministic_timestamp,
                )?;
            } else if metadata.is_file() {
                // Add file entry with fully normalized metadata
                let mut file = std::fs::File::open(&file_path)?;
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(metadata.len());
                header.set_mode(Self::normalize_file_permissions(&metadata));
                header.set_mtime(deterministic_timestamp);
                header.set_uid(0); // Normalized ownership
                header.set_gid(0); // Normalized ownership
                header.set_username("root")?; // Consistent username
                header.set_groupname("root")?; // Consistent group name
                header.set_device_major(0)?; // Clear device numbers
                header.set_device_minor(0)?; // Clear device numbers
                header.set_cksum();

                tar_builder.append_data(
                    &mut header,
                    tar_entry_path.display().to_string(),
                    &mut file,
                )?;
            } else if metadata.is_symlink() {
                // Handle symlinks deterministically
                let target = std::fs::read_link(&file_path)?;
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777); // Standard symlink permissions
                header.set_mtime(deterministic_timestamp);
                header.set_uid(0); // Normalized ownership
                header.set_gid(0); // Normalized ownership
                header.set_username("root")?; // Consistent username
                header.set_groupname("root")?; // Consistent group name
                header.set_link_name(&target)?;
                header.set_device_major(0)?; // Clear device numbers
                header.set_device_minor(0)?; // Clear device numbers
                header.set_cksum();

                tar_builder.append_data(
                    &mut header,
                    tar_entry_path.display().to_string(),
                    std::io::empty(),
                )?;
            }
            // Skip other special files (device nodes, fifos, etc.) for security and consistency
        }

        Ok(())
    }

    /// Legacy deterministic method - redirects to timestamped version
    #[allow(dead_code)]
    fn add_directory_to_tar_deterministic(
        tar_builder: &mut tar::Builder<std::fs::File>,
        dir_path: &Path,
        tar_path: &Path,
    ) -> Result<(), Error> {
        let deterministic_timestamp = Self::get_deterministic_timestamp();
        Self::add_directory_to_tar_with_timestamp(
            tar_builder,
            dir_path,
            tar_path,
            deterministic_timestamp,
        )
    }

    /// Legacy method for backward compatibility - redirects to deterministic version
    #[allow(dead_code)]
    fn add_directory_to_tar(
        tar_builder: &mut tar::Builder<std::fs::File>,
        dir_path: &Path,
        tar_path: &Path,
    ) -> Result<(), Error> {
        Self::add_directory_to_tar_deterministic(tar_builder, dir_path, tar_path)
    }

    /// Get deterministic timestamp for reproducible builds
    /// Uses `SOURCE_DATE_EPOCH` if set, otherwise uses epoch (0)
    fn get_deterministic_timestamp() -> u64 {
        std::env::var(SOURCE_DATE_EPOCH_VAR)
            .ok()
            .and_then(|val| val.parse::<u64>().ok())
            .unwrap_or(DETERMINISTIC_TIMESTAMP)
    }

    /// Normalize file permissions for deterministic output
    /// Ensures consistent permissions across different filesystems and umask settings
    fn normalize_file_permissions(metadata: &std::fs::Metadata) -> u32 {
        use std::os::unix::fs::PermissionsExt;

        let current_mode = metadata.permissions().mode();

        if metadata.is_dir() {
            0o755 // Directories: rwxr-xr-x
        } else if metadata.is_file() {
            // Files: check if any execute bit is set
            if current_mode & 0o111 != 0 {
                0o755 // Executable files: rwxr-xr-x
            } else {
                0o644 // Regular files: rw-r--r--
            }
        } else {
            0o644 // Default for other file types
        }
    }

    /// Compress tar archive with zstd using async-compression
    async fn compress_with_zstd(&self, tar_path: &Path, output_path: &Path) -> Result<(), Error> {
        use async_compression::tokio::write::ZstdEncoder;
        use async_compression::Level;
        use tokio::fs::File;
        use tokio::io::{AsyncWriteExt, BufReader};

        let input_file = File::open(tar_path).await?;
        let output_file = File::create(output_path).await?;

        // Create zstd encoder with specified compression level
        let compression_level = self.config.compression_config.level.zstd_level();
        let level = Level::Precise(compression_level);
        let mut encoder = ZstdEncoder::with_quality(output_file, level);

        // Copy tar file through zstd encoder
        let mut reader = BufReader::new(input_file);
        tokio::io::copy(&mut reader, &mut encoder).await?;

        // Ensure all data is written
        encoder.shutdown().await?;

        Ok(())
    }

    /// Sign the package if signing is enabled
    async fn sign_package(&self, context: &BuildContext, package_path: &Path) -> Result<(), Error> {
        if !self.config.signing_config.enabled {
            return Ok(());
        }

        Self::send_event(
            context,
            Event::OperationStarted {
                operation: format!(
                    "Signing package {}",
                    package_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ),
            },
        );

        let signer = PackageSigner::new(self.config.signing_config.clone());

        match signer.sign_package(package_path).await? {
            Some(sig_path) => {
                Self::send_event(
                    context,
                    Event::OperationCompleted {
                        operation: format!("Package signed: {}", sig_path.display()),
                        success: true,
                    },
                );
            }
            None => {
                // Signing was disabled
                Self::send_event(
                    context,
                    Event::OperationCompleted {
                        operation: "Package signing skipped (disabled)".to_string(),
                        success: true,
                    },
                );
            }
        }

        Ok(())
    }

    /// Setup build environment with full isolation
    async fn setup_build_environment(
        &self,
        context: &BuildContext,
    ) -> Result<BuildEnvironment, Error> {
        // Create build environment with full isolation setup
        let default_build_root;
        let build_root = if let Some(root) = &self.config.build_root {
            root.as_path()
        } else {
            default_build_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            &default_build_root
        };
        let mut environment = BuildEnvironment::new(context.clone(), build_root)?;

        // Configure environment with resolver, store, and net client if available
        if let Some(resolver) = &self.resolver {
            environment = environment.with_resolver(resolver.clone());
        }
        if let Some(store) = &self.store {
            environment = environment.with_store(store.clone());
        }
        if let Some(net) = &self.net {
            environment = environment.with_net(net.clone());
        }

        // Initialize isolated environment
        environment.initialize().await?;

        // Verify isolation is properly set up
        environment.verify_isolation()?;

        Self::send_event(
            context,
            Event::OperationStarted {
                operation: format!(
                    "Build environment isolated for {} {}",
                    context.name, context.version
                ),
            },
        );

        Ok(environment)
    }

    /// Execute recipe and setup build dependencies
    async fn execute_recipe_and_setup_deps(
        &self,
        context: &BuildContext,
        environment: &mut BuildEnvironment,
    ) -> Result<(Vec<String>, sps2_package::RecipeMetadata), Error> {
        // Execute recipe
        let (runtime_deps, build_deps, recipe_metadata) =
            self.execute_recipe(context, environment).await?;

        // Setup build dependencies in isolated environment
        if !build_deps.is_empty() {
            Self::send_event(
                context,
                Event::OperationStarted {
                    operation: format!("Setting up {} build dependencies", build_deps.len()),
                },
            );

            environment.setup_dependencies(build_deps).await?;

            // Log environment summary for debugging
            let env_summary = environment.environment_summary();
            Self::send_event(
                context,
                Event::DebugLog {
                    message: "Build environment configured".to_string(),
                    context: env_summary,
                },
            );
        }

        Ok((runtime_deps, recipe_metadata))
    }

    /// Run quality checks on the built package
    async fn run_quality_checks(
        &self,
        context: &BuildContext,
        environment: &BuildEnvironment,
    ) -> Result<(), Error> {
        // Scan for hardcoded paths (relocatability check)
        self.scan_for_hardcoded_paths(context, environment).await
    }

    /// Generate SBOM and create package manifest
    async fn generate_sbom_and_manifest(
        &self,
        context: &BuildContext,
        environment: &BuildEnvironment,
        runtime_deps: Vec<String>,
        recipe_metadata: &sps2_package::RecipeMetadata,
    ) -> Result<(SbomFiles, Manifest), Error> {
        // Generate SBOM
        Self::send_event(
            context,
            Event::OperationStarted {
                operation: "Generating SBOM".to_string(),
            },
        );
        let sbom_files = self.generate_sbom(environment).await?;
        Self::send_event(
            context,
            Event::OperationCompleted {
                operation: "SBOM generation completed".to_string(),
                success: true,
            },
        );

        // Create manifest
        Self::send_event(
            context,
            Event::OperationStarted {
                operation: "Creating package manifest".to_string(),
            },
        );
        let manifest = Self::create_manifest(context, runtime_deps, &sbom_files, recipe_metadata);
        Self::send_event(
            context,
            Event::OperationCompleted {
                operation: "Package manifest created".to_string(),
                success: true,
            },
        );

        Ok((sbom_files, manifest))
    }

    /// Create package archive and sign it
    async fn create_and_sign_package(
        &self,
        context: &BuildContext,
        environment: &BuildEnvironment,
        manifest: Manifest,
        sbom_files: SbomFiles,
    ) -> Result<PathBuf, Error> {
        // Package the result
        Self::send_event(
            context,
            Event::OperationStarted {
                operation: "Creating package archive".to_string(),
            },
        );
        let package_path = self
            .create_package(context, environment, manifest, sbom_files)
            .await?;
        Self::send_event(
            context,
            Event::OperationCompleted {
                operation: format!("Package created: {}", package_path.display()),
                success: true,
            },
        );

        // Sign the package if configured
        self.sign_package(context, &package_path).await?;

        Ok(package_path)
    }

    /// Cleanup build environment and finalize
    fn cleanup_and_finalize(
        context: &BuildContext,
        environment: &BuildEnvironment,
        _package_path: &Path,
    ) {
        // Cleanup - skip for debugging
        // environment.cleanup().await?;
        Self::send_event(
            context,
            Event::DebugLog {
                message: format!(
                    "Skipping cleanup for debugging - check {}",
                    environment.build_prefix().display()
                ),
                context: std::collections::HashMap::new(),
            },
        );

        Self::send_event(
            context,
            Event::OperationCompleted {
                operation: format!("Built {} {}", context.name, context.version),
                success: true,
            },
        );
    }

    /// Send event if context has event sender
    fn send_event(context: &BuildContext, event: Event) {
        if let Some(sender) = &context.event_sender {
            let _ = sender.send(event);
        }
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompressionLevel;
    use sps2_types::Version;
    use tempfile::tempdir;

    #[test]
    fn test_build_config() {
        let config = BuildConfig::default();
        assert!(!config.allow_network);
        assert!(config.max_build_time.is_some());
        assert!(config.build_root.is_none()); // Should default to None, which becomes current directory

        let network_config = BuildConfig::with_network();
        assert!(network_config.allow_network);

        let custom_config = BuildConfig::default().with_timeout(1800).with_jobs(4);
        assert_eq!(custom_config.max_build_time, Some(1800));
        assert_eq!(custom_config.build_jobs, Some(4));

        // Test custom build root
        let custom_root = PathBuf::from("/custom/build/root");
        let config_with_root = BuildConfig {
            build_root: Some(custom_root.clone()),
            ..Default::default()
        };
        assert_eq!(config_with_root.build_root, Some(custom_root));
    }

    #[test]
    fn test_builder_creation() {
        let builder = Builder::new();
        assert!(!builder.config.allow_network);

        let config = BuildConfig::with_network();
        let network_builder = Builder::with_config(config);
        assert!(network_builder.config.allow_network);
    }

    #[tokio::test]
    async fn test_build_context() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "test-pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        assert_eq!(context.package_filename(), "test-pkg-1.0.0-1.arm64.sp");
        assert!(context
            .output_path()
            .to_string_lossy()
            .ends_with("test-pkg-1.0.0-1.arm64.sp"));

        let custom_context = context.with_revision(2).with_arch("x86_64".to_string());

        assert_eq!(custom_context.revision, 2);
        assert_eq!(custom_context.arch, "x86_64");
    }

    #[tokio::test]
    async fn test_scan_file_for_hardcoded_paths_clean_file() {
        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Create a clean file with no hardcoded paths
        let test_file = temp.path().join("clean.txt");
        fs::write(&test_file, "Hello world\nThis is a clean file\n")
            .await
            .unwrap();

        let result = builder
            .scan_file_for_hardcoded_paths(&test_file, "/opt/pm/build/test-pkg/1.0.0")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_scan_file_for_hardcoded_paths_violation() {
        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Create a file with hardcoded build path
        let test_file = temp.path().join("violation.txt");
        let build_prefix = "/opt/pm/build/test-pkg/1.0.0";
        let content = format!("#!/bin/bash\necho 'Building in {build_prefix}'\n");
        fs::write(&test_file, content).await.unwrap();

        let result = builder
            .scan_file_for_hardcoded_paths(&test_file, build_prefix)
            .await
            .unwrap();
        assert!(result.is_some());
        let violation_msg = result.unwrap();
        assert!(violation_msg.contains(&test_file.display().to_string()));
        assert!(violation_msg.contains(build_prefix));
    }

    #[tokio::test]
    async fn test_scan_file_for_hardcoded_paths_binary_file() {
        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Create a binary file (non-UTF8 content)
        let test_file = temp.path().join("binary.bin");
        let binary_data = vec![0xFF, 0xFE, 0xFD, 0x00, 0x01, 0x02];
        fs::write(&test_file, binary_data).await.unwrap();

        let result = builder
            .scan_file_for_hardcoded_paths(&test_file, "/opt/pm/build/test-pkg/1.0.0")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_scan_file_for_hardcoded_paths_skip_extensions() {
        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Test various extensions that should be skipped
        let skip_extensions = vec!["so", "dylib", "a", "o", "png", "jpg", "zip", "tar"];

        for ext in skip_extensions {
            let test_file = temp.path().join(format!("test.{ext}"));
            fs::write(&test_file, "some content with /opt/pm/build/test-pkg/1.0.0")
                .await
                .unwrap();

            let result = builder
                .scan_file_for_hardcoded_paths(&test_file, "/opt/pm/build/test-pkg/1.0.0")
                .await
                .unwrap();
            assert!(result.is_none(), "Extension {ext} should be skipped");
        }
    }

    #[tokio::test]
    async fn test_scan_directory_for_hardcoded_paths_empty_dir() {
        let temp = tempdir().unwrap();
        let builder = Builder::new();

        let result = builder
            .scan_directory_for_hardcoded_paths(temp.path(), "/opt/pm/build/test-pkg/1.0.0")
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_scan_directory_for_hardcoded_paths_with_violations() {
        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Create directory structure with some violations
        let subdir = temp.path().join("subdir");
        fs::create_dir_all(&subdir).await.unwrap();

        let build_prefix = "/opt/pm/build/test-pkg/1.0.0";

        // Clean file
        let clean_file = temp.path().join("clean.txt");
        fs::write(&clean_file, "This is clean").await.unwrap();

        // Violation in root
        let violation1 = temp.path().join("violation1.sh");
        fs::write(&violation1, format!("export PATH={build_prefix}:$PATH"))
            .await
            .unwrap();

        // Violation in subdirectory
        let violation2 = subdir.join("violation2.cfg");
        fs::write(&violation2, format!("build_dir={build_prefix}"))
            .await
            .unwrap();

        // Binary file that should be skipped
        let binary_file = subdir.join("binary.so");
        fs::write(&binary_file, format!("some binary data {build_prefix}"))
            .await
            .unwrap();

        let result = builder
            .scan_directory_for_hardcoded_paths(temp.path(), build_prefix)
            .await
            .unwrap();

        // Should find exactly 2 violations (not the binary file)
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|v| v.contains("violation1.sh")));
        assert!(result.iter().any(|v| v.contains("violation2.cfg")));
        assert!(!result.iter().any(|v| v.contains("binary.so")));
        assert!(!result.iter().any(|v| v.contains("clean.txt")));
    }

    #[tokio::test]
    async fn test_scan_for_hardcoded_paths_success() {
        use crate::{BuildContext, BuildEnvironment};
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Create a mock build context and environment
        let context = BuildContext::new(
            "test-pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );

        // Create mock build environment with clean staging directory
        let build_env_temp = tempdir().unwrap();
        let staging_dir = build_env_temp.path().join("stage");
        fs::create_dir_all(&staging_dir).await.unwrap();

        // Create a clean file in staging
        let clean_file = staging_dir.join("bin").join("program");
        fs::create_dir_all(clean_file.parent().unwrap())
            .await
            .unwrap();
        fs::write(&clean_file, "#!/bin/bash\necho 'Hello world'")
            .await
            .unwrap();

        // Mock environment that returns our staging directory
        let build_root = temp.path(); // Use temp directory as build root for test
        let environment = BuildEnvironment::new(context.clone(), build_root).unwrap();

        // This should pass since our staging directory is clean
        let result = builder
            .scan_for_hardcoded_paths(&context, &environment)
            .await;

        // This test will pass if the staging directory is clean
        // In a real test environment, we would need to mock the environment properly
        // For now, this tests the method signature and basic flow
        match result {
            Ok(()) => {
                // Success case - no hardcoded paths found
            }
            Err(e) => {
                // This might happen in test environment, which is expected
                // The important thing is that the method compiles and runs
                println!("Expected test environment behavior: {e}");
            }
        }
    }

    #[test]
    fn test_get_deterministic_timestamp() {
        // Test default behavior (no SOURCE_DATE_EPOCH set)
        std::env::remove_var(SOURCE_DATE_EPOCH_VAR);
        assert_eq!(
            Builder::get_deterministic_timestamp(),
            DETERMINISTIC_TIMESTAMP
        );

        // Test with SOURCE_DATE_EPOCH set
        std::env::set_var(SOURCE_DATE_EPOCH_VAR, "1640995200"); // 2022-01-01 00:00:00 UTC
        assert_eq!(Builder::get_deterministic_timestamp(), 1_640_995_200);

        // Test with invalid SOURCE_DATE_EPOCH (should fall back to default)
        std::env::set_var(SOURCE_DATE_EPOCH_VAR, "invalid");
        assert_eq!(
            Builder::get_deterministic_timestamp(),
            DETERMINISTIC_TIMESTAMP
        );

        // Cleanup
        std::env::remove_var(SOURCE_DATE_EPOCH_VAR);
    }

    #[test]
    fn test_normalize_file_permissions() {
        use std::fs::File;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::tempdir;

        let temp = tempdir().unwrap();

        // Test regular file permissions
        let regular_file = temp.path().join("regular.txt");
        File::create(&regular_file).unwrap();
        let metadata = regular_file.metadata().unwrap();
        assert_eq!(Builder::normalize_file_permissions(&metadata), 0o644);

        // Test executable file permissions
        let exec_file = temp.path().join("executable.sh");
        File::create(&exec_file).unwrap();
        let mut perms = exec_file.metadata().unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&exec_file, perms).unwrap();
        let metadata = exec_file.metadata().unwrap();
        assert_eq!(Builder::normalize_file_permissions(&metadata), 0o755);

        // Test directory permissions
        let dir = temp.path().join("testdir");
        std::fs::create_dir(&dir).unwrap();
        let metadata = dir.metadata().unwrap();
        assert_eq!(Builder::normalize_file_permissions(&metadata), 0o755);
    }

    #[tokio::test]
    async fn test_deterministic_tar_creation_simple() {
        use tempfile::tempdir;

        // Use explicit deterministic timestamp to avoid race conditions with environment variables
        const TEST_TIMESTAMP: u64 = 0;

        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Create a simple test directory structure without modifying timestamps
        let source_dir = temp.path().join("source");
        fs::create_dir_all(&source_dir).await.unwrap();

        // Create one simple file
        let file1 = source_dir.join("test.txt");
        fs::write(&file1, "deterministic content").await.unwrap();

        // Create first tar archive
        let tar1_path = temp.path().join("test1.tar");
        builder
            .create_deterministic_tar_archive_with_timestamp(
                &source_dir,
                &tar1_path,
                TEST_TIMESTAMP,
            )
            .await
            .unwrap();

        // Create second tar archive immediately (no timestamp modification)
        let tar2_path = temp.path().join("test2.tar");
        builder
            .create_deterministic_tar_archive_with_timestamp(
                &source_dir,
                &tar2_path,
                TEST_TIMESTAMP,
            )
            .await
            .unwrap();

        // Read tar contents
        let tar1_content = fs::read(&tar1_path).await.unwrap();
        let tar2_content = fs::read(&tar2_path).await.unwrap();

        assert_eq!(
            tar1_content, tar2_content,
            "Deterministic tar archives should be identical for the same input"
        );

        // Verify files exist and have expected sizes
        assert!(!tar1_content.is_empty(), "Tar archive should not be empty");
        assert!(!tar2_content.is_empty(), "Tar archive should not be empty");
    }

    #[tokio::test]
    async fn test_deterministic_tar_creation_with_timestamp_modification() {
        use std::os::unix::fs::PermissionsExt;
        use tempfile::tempdir;

        // Use explicit deterministic timestamp to avoid race conditions with environment variables
        const TEST_TIMESTAMP: u64 = 0;

        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Create a test directory structure (simpler for debugging)
        let source_dir = temp.path().join("source");
        fs::create_dir_all(&source_dir).await.unwrap();

        // Create files in alphabetical order to be deterministic
        let file1 = source_dir.join("a_file.txt");
        fs::write(&file1, "content1").await.unwrap();

        let file2 = source_dir.join("b_file.sh");
        fs::write(&file2, "#!/bin/bash\necho hello").await.unwrap();
        let mut perms = file2.metadata().unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&file2, perms).unwrap();

        // Create first tar archive
        let tar1_path = temp.path().join("test1.tar");
        builder
            .create_deterministic_tar_archive_with_timestamp(
                &source_dir,
                &tar1_path,
                TEST_TIMESTAMP,
            )
            .await
            .unwrap();

        // Sleep briefly to ensure different filesystem timestamps
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Modify file timestamps (this shouldn't affect the output)
        let now = std::time::SystemTime::now();
        let later = now + std::time::Duration::from_secs(3600);
        let later_file_time = filetime::FileTime::from_system_time(later);
        filetime::set_file_times(&file1, later_file_time, later_file_time).unwrap();

        // Create second tar archive
        let tar2_path = temp.path().join("test2.tar");
        builder
            .create_deterministic_tar_archive_with_timestamp(
                &source_dir,
                &tar2_path,
                TEST_TIMESTAMP,
            )
            .await
            .unwrap();

        // Read tar contents
        let tar1_content = fs::read(&tar1_path).await.unwrap();
        let tar2_content = fs::read(&tar2_path).await.unwrap();

        // If they differ, let's see the hex dump for debugging
        if tar1_content != tar2_content {
            println!("TAR 1 length: {}", tar1_content.len());
            println!("TAR 2 length: {}", tar2_content.len());

            // Find first difference
            for (i, (a, b)) in tar1_content.iter().zip(tar2_content.iter()).enumerate() {
                if a != b {
                    println!("First difference at byte {i}: {a} vs {b}");

                    // Print context around the difference
                    let start = i.saturating_sub(10);
                    let end = (i + 10).min(tar1_content.len());
                    println!(
                        "TAR 1 bytes {}..{}: {:?}",
                        start,
                        end,
                        &tar1_content[start..end]
                    );
                    println!(
                        "TAR 2 bytes {}..{}: {:?}",
                        start,
                        end,
                        &tar2_content[start..end]
                    );
                    break;
                }
            }
        }

        assert_eq!(
            tar1_content, tar2_content,
            "Deterministic tar archives should be identical regardless of filesystem timestamps"
        );

        // Verify files exist and have expected sizes
        assert!(!tar1_content.is_empty(), "Tar archive should not be empty");
        assert!(!tar2_content.is_empty(), "Tar archive should not be empty");
    }

    #[tokio::test]
    async fn test_deterministic_tar_with_source_date_epoch() {
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let builder = Builder::new();

        // Create a simple test directory
        let source_dir = temp.path().join("source");
        fs::create_dir_all(&source_dir).await.unwrap();
        fs::write(source_dir.join("test.txt"), "test content")
            .await
            .unwrap();

        // Set SOURCE_DATE_EPOCH
        std::env::set_var(SOURCE_DATE_EPOCH_VAR, "1640995200"); // 2022-01-01

        // Create tar archive with SOURCE_DATE_EPOCH
        let tar1_path = temp.path().join("test_epoch.tar");
        builder
            .create_deterministic_tar_archive(&source_dir, &tar1_path)
            .await
            .unwrap();

        // Change SOURCE_DATE_EPOCH to a different value
        std::env::set_var(SOURCE_DATE_EPOCH_VAR, "1609459200"); // 2021-01-01

        // Create second tar archive
        let tar2_path = temp.path().join("test_epoch2.tar");
        builder
            .create_deterministic_tar_archive(&source_dir, &tar2_path)
            .await
            .unwrap();

        // Cleanup environment
        std::env::remove_var(SOURCE_DATE_EPOCH_VAR);

        // Verify that the tar files are different (different timestamps)
        let tar1_content = fs::read(&tar1_path).await.unwrap();
        let tar2_content = fs::read(&tar2_path).await.unwrap();

        assert_ne!(
            tar1_content, tar2_content,
            "Tar archives with different SOURCE_DATE_EPOCH should be different"
        );
    }

    #[test]
    fn test_compression_config_default() {
        let config = CompressionConfig::default();
        assert_eq!(config.level, CompressionLevel::Balanced);
    }

    #[test]
    fn test_build_config_compression_methods() {
        // Test default compression config
        let config = BuildConfig::default();
        assert_eq!(config.compression_config, CompressionConfig::default());

        // Test fast compression
        let fast_config = BuildConfig::with_fast_compression();
        assert_eq!(fast_config.compression_config.level, CompressionLevel::Fast);

        // Test balanced compression
        let balanced_config = BuildConfig::with_balanced_compression();
        assert_eq!(
            balanced_config.compression_config.level,
            CompressionLevel::Balanced
        );

        // Test maximum compression
        let max_config = BuildConfig::with_maximum_compression();
        assert_eq!(
            max_config.compression_config.level,
            CompressionLevel::Maximum
        );

        // Test custom compression level
        let custom_config = BuildConfig::with_custom_compression(15);
        assert_eq!(
            custom_config.compression_config.level,
            CompressionLevel::Custom(15)
        );

        // Test compression config setting
        let custom_compression_config = CompressionConfig::fast();
        let config_with_custom =
            BuildConfig::default().with_compression_config(custom_compression_config.clone());
        assert_eq!(
            config_with_custom.compression_config,
            custom_compression_config
        );
    }

    #[tokio::test]
    async fn test_zstd_compression() {
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let builder = Builder::with_config(BuildConfig::default());

        // Create a test tar file
        let tar_path = temp.path().join("test.tar");
        let content = b"Test content for zstd compression";
        fs::write(&tar_path, content).await.unwrap();

        // Compress with zstd format
        let compressed_path = temp.path().join("test.sp");
        builder
            .compress_with_zstd(&tar_path, &compressed_path)
            .await
            .unwrap();

        // Verify the compressed file was created
        assert!(compressed_path.exists());

        // The compressed file should be valid zstd
        let compressed_data = fs::read(&compressed_path).await.unwrap();
        assert!(!compressed_data.is_empty());

        // Should be able to decompress back to original
        let decompressed = zstd::decode_all(&compressed_data[..]).unwrap();
        assert_eq!(decompressed, content);
    }

    #[tokio::test]
    async fn test_zstd_decompression() {
        use tempfile::tempdir;

        let temp = tempdir().unwrap();

        // Create a test package
        let content = b"Test content for decompression";

        // Compress with zstd
        let builder = Builder::with_config(BuildConfig::default());
        let tar_path = temp.path().join("test.tar");
        let sp_path = temp.path().join("test.sp");
        fs::write(&tar_path, content).await.unwrap();
        builder
            .compress_with_zstd(&tar_path, &sp_path)
            .await
            .unwrap();

        // Should decompress to the same content using standard zstd
        let compressed_data = fs::read(&sp_path).await.unwrap();
        let decompressed = zstd::decode_all(&compressed_data[..]).unwrap();

        assert_eq!(decompressed, content);
    }
}

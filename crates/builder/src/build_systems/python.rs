//! Python build system implementation with PEP 517/518 support

use super::{BuildSystem, BuildSystemConfig, BuildSystemContext, TestFailure, TestResults};
use async_trait::async_trait;
use sps2_errors::{BuildError, Error};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Python build system with PEP 517/518 compliance
pub struct PythonBuildSystem {
    config: BuildSystemConfig,
}

impl PythonBuildSystem {
    /// Create a new Python build system instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: BuildSystemConfig {
                supports_out_of_source: true,
                supports_parallel_builds: false, // Most Python builds are sequential
                supports_incremental_builds: true,
                default_configure_args: vec![],
                default_build_args: vec![],
                env_prefix: Some("PYTHON_".to_string()),
                watch_patterns: vec![
                    "setup.py".to_string(),
                    "setup.cfg".to_string(),
                    "pyproject.toml".to_string(),
                    "requirements.txt".to_string(),
                    "**/*.py".to_string(),
                ],
            },
        }
    }

    /// Detect build backend from pyproject.toml
    async fn detect_build_backend(&self, source_dir: &Path) -> Result<BuildBackend, Error> {
        let pyproject_path = source_dir.join("pyproject.toml");

        if pyproject_path.exists() {
            // Read pyproject.toml to detect build backend
            let content = fs::read_to_string(&pyproject_path).await?;

            // Simple parsing - in production would use toml crate
            if content.contains("[build-system]") {
                if content.contains("setuptools") {
                    return Ok(BuildBackend::Setuptools);
                } else if content.contains("poetry") {
                    return Ok(BuildBackend::Poetry);
                } else if content.contains("flit") {
                    return Ok(BuildBackend::Flit);
                } else if content.contains("hatchling") || content.contains("hatch") {
                    return Ok(BuildBackend::Hatch);
                } else if content.contains("pdm") {
                    return Ok(BuildBackend::Pdm);
                } else if content.contains("maturin") {
                    return Ok(BuildBackend::Maturin);
                }
                // Generic PEP 517 backend
                return Ok(BuildBackend::Pep517);
            }
        }

        // Fall back to setup.py
        if source_dir.join("setup.py").exists() {
            return Ok(BuildBackend::SetupPy);
        }

        Err(BuildError::ConfigureFailed {
            message: "No Python build configuration found".to_string(),
        }
        .into())
    }

    /// Check if uv is available
    async fn check_uv_available(&self, ctx: &BuildSystemContext) -> Result<bool, Error> {
        let result = ctx.execute("uv", &["--version"], None).await;
        Ok(result.map(|r| r.success).unwrap_or(false))
    }

    /// Create virtual environment for isolated builds
    async fn create_venv(&self, ctx: &BuildSystemContext) -> Result<PathBuf, Error> {
        let venv_path = ctx.build_dir.join("venv");
        let use_uv = self.check_uv_available(ctx).await?;

        if use_uv {
            // Try uv for faster venv creation
            let result = ctx
                .execute(
                    "uv",
                    &["venv", "--seed", &venv_path.display().to_string()],
                    Some(&ctx.source_dir),
                )
                .await?;

            if result.success {
                return Ok(venv_path);
            }
            // If uv failed, fall back to standard venv
        }

        // Fall back to standard venv (either uv not available or failed)
        let result = ctx
            .execute(
                "python3",
                &["-m", "venv", &venv_path.display().to_string()],
                Some(&ctx.source_dir),
            )
            .await?;

        if !result.success {
            return Err(BuildError::ConfigureFailed {
                message: format!("Failed to create virtual environment: {}", result.stderr),
            }
            .into());
        }

        Ok(venv_path)
    }

    /// Get pip install arguments
    fn get_pip_args(ctx: &BuildSystemContext, user_args: &[String]) -> Vec<String> {
        let mut args = vec!["install".to_string()];

        // Add non-interactive flags to prevent prompting
        args.push("--no-input".to_string());
        args.push("--disable-pip-version-check".to_string());

        // Add offline mode if network is disabled
        if !ctx.network_allowed {
            args.push("--no-index".to_string());
            args.push("--find-links".to_string());
            args.push(ctx.source_dir.join("vendor").display().to_string());
        }

        // Add prefix for installation - use clean python/[package] structure
        args.push("--prefix".to_string());
        let staging_dir = ctx.env.staging_dir();
        let live_prefix = ctx.env.get_live_prefix().trim_start_matches('/');
        let package_name = ctx.env.package_name();
        let package_specific_prefix = staging_dir
            .join(live_prefix)
            .join("python")
            .join(package_name);
        args.push(package_specific_prefix.display().to_string());

        // Install with dependencies for self-contained packages

        // Add user arguments
        args.extend(user_args.iter().cloned());

        args
    }

    /// Build wheel using uv
    async fn build_wheel_uv(&self, ctx: &BuildSystemContext) -> Result<PathBuf, Error> {
        let wheel_dir = ctx.build_dir.join("dist");
        fs::create_dir_all(&wheel_dir).await?;

        // Build wheel using uv build command
        let result = ctx
            .execute(
                "uv",
                &[
                    "build",
                    "--wheel",
                    "--out-dir",
                    &wheel_dir.display().to_string(),
                ],
                Some(&ctx.source_dir),
            )
            .await?;

        if !result.success {
            return Err(BuildError::CompilationFailed {
                message: format!("Failed to build wheel with uv: {}", result.stderr),
            }
            .into());
        }

        // Find the built wheel
        self.find_wheel_in_dir(&wheel_dir).await
    }

    /// Build wheel using PEP 517
    async fn build_wheel_pep517(&self, ctx: &BuildSystemContext) -> Result<PathBuf, Error> {
        let wheel_dir = ctx.build_dir.join("dist");
        fs::create_dir_all(&wheel_dir).await?;

        // Get venv path from environment
        let venv_path = if let Ok(extra_env) = ctx.extra_env.read() {
            extra_env
                .get("PYTHON_VENV_PATH")
                .map_or_else(|| ctx.build_dir.join("venv"), PathBuf::from)
        } else {
            ctx.build_dir.join("venv")
        };

        // Use venv's pip
        let pip_path = venv_path.join("bin/pip");

        // Install build dependencies
        let result = ctx
            .execute(
                &pip_path.display().to_string(),
                &[
                    "install",
                    "--upgrade",
                    "--no-input", // Prevent interactive prompts
                    "pip",
                    "setuptools",
                    "wheel",
                    "build",
                ],
                Some(&ctx.source_dir),
            )
            .await?;

        if !result.success {
            return Err(BuildError::ConfigureFailed {
                message: format!("Failed to install build tools: {}", result.stderr),
            }
            .into());
        }

        // Use venv's python
        let python_path = venv_path.join("bin/python3");

        // Build wheel using python-build
        let wheel_dir_str = wheel_dir.display().to_string();
        let mut build_args = vec!["-m", "build", "--wheel", "--outdir", &wheel_dir_str];

        if !ctx.network_allowed {
            build_args.push("--no-isolation");
        }

        let result = ctx
            .execute(
                &python_path.display().to_string(),
                &build_args,
                Some(&ctx.source_dir),
            )
            .await?;

        if !result.success {
            return Err(BuildError::CompilationFailed {
                message: format!("Failed to build wheel: {}", result.stderr),
            }
            .into());
        }

        // Find the built wheel
        self.find_wheel_in_dir(&wheel_dir).await
    }

    /// Find wheel file in directory
    async fn find_wheel_in_dir(&self, dir: &Path) -> Result<PathBuf, Error> {
        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("whl") {
                return Ok(path);
            }
        }

        Err(BuildError::CompilationFailed {
            message: "No wheel file found after build".to_string(),
        }
        .into())
    }

    /// Parse pytest output
    fn parse_pytest_output(output: &str) -> (usize, usize, usize, Vec<TestFailure>) {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut failures = vec![];

        for line in output.lines() {
            // Look for summary line like "====== 5 passed, 2 failed, 1 skipped in 1.23s ======"
            if line.contains("passed") || line.contains("failed") || line.contains("skipped") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                for (i, part) in parts.iter().enumerate() {
                    if let Ok(num) = part.parse::<usize>() {
                        if i + 1 < parts.len() {
                            if let Some(next_part) = parts.get(i + 1) {
                                match *next_part {
                                    "passed" | "passed," => passed = num,
                                    "failed" | "failed," => failed = num,
                                    "skipped" | "skipped," => skipped = num,
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            // Capture individual test failures
            else if line.starts_with("FAILED ") {
                if let Some(test_info) = line.strip_prefix("FAILED ") {
                    let test_name = test_info.split(" - ").next().unwrap_or(test_info);
                    failures.push(TestFailure {
                        name: test_name.to_string(),
                        message: line.to_string(),
                        details: None,
                    });
                }
            }
        }

        let total = passed + failed + skipped;
        (total, passed, failed, failures)
    }
}

/// Python build backend types
#[derive(Debug, Clone)]
enum BuildBackend {
    SetupPy,    // Legacy setup.py
    Setuptools, // Modern setuptools with pyproject.toml
    Poetry,     // Poetry build system
    Flit,       // Flit build system
    Hatch,      // Hatch/Hatchling build system
    Pdm,        // PDM build system
    Maturin,    // Maturin for Rust extensions
    Pep517,     // Generic PEP 517 backend
}

impl PythonBuildSystem {
    /// Generate lockfile with graceful fallback from uv to pip-compile
    async fn generate_lockfile_with_fallback(
        &self,
        ctx: &BuildSystemContext,
    ) -> Result<PathBuf, Error> {
        let lockfile_path = ctx.build_dir.join("requirements.lock.txt");

        // Try uv first if available
        if self.check_uv_available(ctx).await? {
            if let Ok(path) = self.generate_lockfile_uv(ctx, &lockfile_path).await {
                return Ok(path);
            }
            // If uv failed, fall through to pip-compile fallback
        }

        // Fall back to pip-compile
        self.generate_lockfile_pip_compile(ctx, &lockfile_path)
            .await
    }

    /// Generate lockfile using uv
    async fn generate_lockfile_uv(
        &self,
        ctx: &BuildSystemContext,
        lockfile_path: &std::path::Path,
    ) -> Result<PathBuf, Error> {
        // Try pyproject.toml first
        let result = ctx
            .execute(
                "uv",
                &[
                    "pip",
                    "compile",
                    "--output-file",
                    &lockfile_path.display().to_string(),
                    "pyproject.toml",
                ],
                Some(&ctx.source_dir),
            )
            .await?;

        if result.success {
            return Ok(lockfile_path.to_path_buf());
        }

        // Try requirements.txt if pyproject.toml fails
        let req_txt = ctx.source_dir.join("requirements.txt");
        if req_txt.exists() {
            let result = ctx
                .execute(
                    "uv",
                    &[
                        "pip",
                        "compile",
                        "--output-file",
                        &lockfile_path.display().to_string(),
                        "requirements.txt",
                    ],
                    Some(&ctx.source_dir),
                )
                .await?;

            if result.success {
                return Ok(lockfile_path.to_path_buf());
            }
        }

        Err(BuildError::CompilationFailed {
            message: "Failed to generate lockfile with uv".to_string(),
        }
        .into())
    }

    /// Generate lockfile using pip-compile
    async fn generate_lockfile_pip_compile(
        &self,
        ctx: &BuildSystemContext,
        lockfile_path: &std::path::Path,
    ) -> Result<PathBuf, Error> {
        let venv_path = if let Ok(extra_env) = ctx.extra_env.read() {
            extra_env
                .get("PYTHON_VENV_PATH")
                .map_or_else(|| ctx.build_dir.join("venv"), std::path::PathBuf::from)
        } else {
            ctx.build_dir.join("venv")
        };

        let pip_path = venv_path.join("bin/pip");

        // Install pip-tools
        let _ = ctx
            .execute(
                &pip_path.display().to_string(),
                &["install", "pip-tools"],
                Some(&ctx.source_dir),
            )
            .await?;

        // Use pip-compile
        let pip_compile = venv_path.join("bin/pip-compile");
        let result = ctx
            .execute(
                &pip_compile.display().to_string(),
                &[
                    "--output-file",
                    &lockfile_path.display().to_string(),
                    "pyproject.toml",
                ],
                Some(&ctx.source_dir),
            )
            .await?;

        if !result.success {
            return Err(BuildError::CompilationFailed {
                message: format!("Failed to generate lockfile: {}", result.stderr),
            }
            .into());
        }

        Ok(lockfile_path.to_path_buf())
    }

    /// Extract entry points from wheel
    fn extract_entry_points(wheel_path: &Path) -> Result<HashMap<String, String>, Error> {
        use std::io::Read;
        use zip::ZipArchive;

        let mut executables = HashMap::new();

        let file = std::fs::File::open(wheel_path).map_err(|e| BuildError::CompilationFailed {
            message: format!("Failed to open wheel file: {e}"),
        })?;

        let mut archive = ZipArchive::new(file).map_err(|e| BuildError::CompilationFailed {
            message: format!("Failed to read wheel archive: {e}"),
        })?;

        // Find .dist-info/entry_points.txt
        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| BuildError::CompilationFailed {
                    message: format!("Failed to read wheel entry: {e}"),
                })?;

            if file.name().ends_with(".dist-info/entry_points.txt") {
                let mut contents = String::new();
                file.read_to_string(&mut contents)
                    .map_err(|e| BuildError::CompilationFailed {
                        message: format!("Failed to read entry_points.txt: {e}"),
                    })?;

                // Parse entry points
                let mut in_console_scripts = false;
                for line in contents.lines() {
                    let trimmed = line.trim();
                    if trimmed == "[console_scripts]" {
                        in_console_scripts = true;
                        continue;
                    }
                    if trimmed.starts_with('[') {
                        in_console_scripts = false;
                        continue;
                    }
                    if in_console_scripts && trimmed.contains('=') {
                        let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
                        if parts.len() == 2 {
                            executables
                                .insert(parts[0].trim().to_string(), parts[1].trim().to_string());
                        }
                    }
                }
                break;
            }
        }

        Ok(executables)
    }

    /// Extract Python version requirement from pyproject.toml
    async fn extract_requires_python(&self, source_dir: &Path) -> Result<String, Error> {
        let pyproject_path = source_dir.join("pyproject.toml");

        if pyproject_path.exists() {
            let content = fs::read_to_string(&pyproject_path).await?;

            // Simple parsing - look for requires-python
            for line in content.lines() {
                if line.contains("requires-python") {
                    if let Some(value) = line.split('=').nth(1) {
                        // Remove quotes and whitespace
                        let cleaned = value.trim().trim_matches('"').trim_matches('\'');
                        return Ok(cleaned.to_string());
                    }
                }
            }
        }

        // Default to current Python 3 requirement
        Ok(">=3.8".to_string())
    }

    /// Remove `direct_url.json` files that contain hardcoded paths
    async fn remove_direct_url_files(&self, prefix_path: &Path) -> Result<(), Error> {
        let lib_dir = prefix_path.join("lib");
        if !lib_dir.exists() {
            return Ok(());
        }

        let mut stack = vec![lib_dir];
        while let Some(dir) = stack.pop() {
            let mut entries = fs::read_dir(&dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path.clone());
                    // Check for dist-info directories
                    if path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".dist-info"))
                    {
                        let direct_url = path.join("direct_url.json");
                        if direct_url.exists() {
                            fs::remove_file(&direct_url).await?;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for PythonBuildSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BuildSystem for PythonBuildSystem {
    async fn detect(&self, source_dir: &Path) -> Result<bool, Error> {
        // Check for pyproject.toml (PEP 517/518)
        if source_dir.join("pyproject.toml").exists() {
            return Ok(true);
        }

        // Check for setup.py (legacy)
        if source_dir.join("setup.py").exists() {
            return Ok(true);
        }

        // Check for setup.cfg
        if source_dir.join("setup.cfg").exists() {
            return Ok(true);
        }

        Ok(false)
    }

    fn get_config_options(&self) -> BuildSystemConfig {
        self.config.clone()
    }

    async fn configure(&self, ctx: &BuildSystemContext, _args: &[String]) -> Result<(), Error> {
        // Detect build backend
        let backend = self.detect_build_backend(&ctx.source_dir).await?;

        // Verify Python is available
        let result = ctx.execute("python3", &["--version"], None).await?;
        if !result.success {
            return Err(BuildError::ConfigureFailed {
                message: "python3 not found in PATH".to_string(),
            }
            .into());
        }

        // Always create virtual environment for isolation
        let venv_path = self.create_venv(ctx).await?;

        // Store detected backend and venv path in environment for later use
        if let Ok(mut extra_env) = ctx.extra_env.write() {
            extra_env.insert("PYTHON_BUILD_BACKEND".to_string(), format!("{backend:?}"));
            extra_env.insert(
                "PYTHON_VENV_PATH".to_string(),
                venv_path.display().to_string(),
            );
        }

        Ok(())
    }

    async fn build(&self, ctx: &BuildSystemContext, _args: &[String]) -> Result<(), Error> {
        // Try uv first if available, with graceful fallback to PEP 517
        let wheel_path = if self.check_uv_available(ctx).await? {
            match self.build_wheel_uv(ctx).await {
                Ok(path) => path,
                Err(_) => {
                    // If uv build failed, fall back to PEP 517
                    self.build_wheel_pep517(ctx).await?
                }
            }
        } else {
            self.build_wheel_pep517(ctx).await?
        };

        // Generate lockfile with graceful fallback
        let lockfile_path = self.generate_lockfile_with_fallback(ctx).await?;

        // Extract entry points from wheel
        let entry_points = Self::extract_entry_points(&wheel_path)?;

        // Extract Python version requirement
        let requires_python = self.extract_requires_python(&ctx.source_dir).await?;

        // Store all metadata for install phase
        if let Ok(mut extra_env) = ctx.extra_env.write() {
            extra_env.insert(
                "PYTHON_WHEEL_PATH".to_string(),
                wheel_path.display().to_string(),
            );
            extra_env.insert(
                "PYTHON_LOCKFILE_PATH".to_string(),
                lockfile_path.display().to_string(),
            );
            extra_env.insert(
                "PYTHON_ENTRY_POINTS".to_string(),
                serde_json::to_string(&entry_points).unwrap_or_else(|_| "{}".to_string()),
            );
            extra_env.insert("PYTHON_REQUIRES_VERSION".to_string(), requires_python);
        }

        Ok(())
    }

    async fn test(&self, ctx: &BuildSystemContext) -> Result<TestResults, Error> {
        let start = std::time::Instant::now();

        // Try pytest first
        let mut test_cmd = "pytest";
        let test_args;
        let jobs_str;

        // Check if pytest is available
        let pytest_check = ctx.execute("pytest", &["--version"], None).await;

        if pytest_check.map(|r| r.success).unwrap_or(false) {
            // Use pytest with verbose output
            let mut args = vec!["-v".to_string(), "--tb=short".to_string()];

            // Add parallel execution if supported
            if ctx.jobs > 1 {
                args.push("-n".to_string());
                jobs_str = ctx.jobs.to_string();
                args.push(jobs_str.clone());
            }
            test_args = args;
        } else {
            // Fall back to unittest
            test_cmd = "python3";
            test_args = vec![
                "-m".to_string(),
                "unittest".to_string(),
                "discover".to_string(),
            ];
        }

        // Run tests
        let test_arg_refs: Vec<&str> = test_args.iter().map(String::as_str).collect();
        let result = ctx
            .execute(test_cmd, &test_arg_refs, Some(&ctx.source_dir))
            .await?;

        let duration = start.elapsed().as_secs_f64();
        let output = format!("{}\n{}", result.stdout, result.stderr);

        // Parse test results
        let (total, passed, failed, failures) = if test_cmd == "pytest" {
            Self::parse_pytest_output(&output)
        } else {
            // Simple parsing for unittest
            if result.success {
                (1, 1, 0, vec![])
            } else {
                (
                    1,
                    0,
                    1,
                    vec![TestFailure {
                        name: "unittest".to_string(),
                        message: "Tests failed".to_string(),
                        details: Some(output.clone()),
                    }],
                )
            }
        };

        Ok(TestResults {
            total,
            passed,
            failed,
            skipped: 0,
            duration,
            output,
            failures,
        })
    }

    async fn install(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        // Get wheel path and venv path from build phase
        let (wheel_path, venv_path) = if let Ok(extra_env) = ctx.extra_env.read() {
            let wheel = extra_env.get("PYTHON_WHEEL_PATH").cloned().ok_or_else(|| {
                BuildError::InstallFailed {
                    message: "No wheel found from build phase".to_string(),
                }
            })?;
            let venv = extra_env
                .get("PYTHON_VENV_PATH")
                .map_or_else(|| ctx.build_dir.join("venv"), PathBuf::from);
            (wheel, venv)
        } else {
            return Err(BuildError::InstallFailed {
                message: "Cannot access extra environment".to_string(),
            }
            .into());
        };

        // Use venv's pip
        let pip_path = venv_path.join("bin/pip");

        // Install wheel using pip
        let pip_args = Self::get_pip_args(ctx, &[wheel_path.clone()]);
        let arg_refs: Vec<&str> = pip_args.iter().map(String::as_str).collect();

        let result = ctx
            .execute(
                &pip_path.display().to_string(),
                &arg_refs,
                Some(&ctx.source_dir),
            )
            .await?;

        if !result.success {
            return Err(BuildError::InstallFailed {
                message: format!("pip install failed: {}", result.stderr),
            }
            .into());
        }

        // Fix shebangs to point to the correct packaged Python version
        let staging_dir = ctx.env.staging_dir();
        let live_prefix = ctx.env.get_live_prefix().trim_start_matches('/');
        let package_name = ctx.env.package_name();
        let package_specific_prefix = staging_dir
            .join(live_prefix)
            .join("python")
            .join(package_name);
        let scripts_dir = package_specific_prefix.join("bin");
        if scripts_dir.exists() {
            self.fix_shebangs(&scripts_dir, &package_specific_prefix, ctx)
                .await?;
        }

        // Remove direct_url.json files which contain hardcoded paths
        self.remove_direct_url_files(&package_specific_prefix)
            .await?;

        Ok(())
    }

    fn get_env_vars(&self, ctx: &BuildSystemContext) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        // Set PYTHONPATH to include staging directory with LIVE_PREFIX
        let staging_dir = ctx.env.staging_dir();
        let prefix_in_staging = staging_dir.join(ctx.env.get_live_prefix().trim_start_matches('/'));
        let site_packages = prefix_in_staging.join("lib/python*/site-packages");
        vars.insert(
            "PYTHONPATH".to_string(),
            site_packages.display().to_string(),
        );

        // Disable user site packages for isolation
        vars.insert("PYTHONNOUSERSITE".to_string(), "1".to_string());

        // Set pip configuration
        vars.insert("PIP_DISABLE_PIP_VERSION_CHECK".to_string(), "1".to_string());
        vars.insert("PIP_NO_WARN_SCRIPT_LOCATION".to_string(), "1".to_string());

        // Use virtual environment if created
        let venv_path = ctx.build_dir.join("venv");
        if venv_path.exists() {
            let venv_bin = venv_path.join("bin");
            if let Some(path) = vars.get("PATH") {
                vars.insert(
                    "PATH".to_string(),
                    format!("{}:{}", venv_bin.display(), path),
                );
            }
            vars.insert("VIRTUAL_ENV".to_string(), venv_path.display().to_string());
        }

        vars
    }

    fn name(&self) -> &'static str {
        "python"
    }
}

impl PythonBuildSystem {
    /// Detect Python version from site-packages directory structure
    async fn detect_python_version(&self, prefix_in_staging: &Path) -> Result<String, Error> {
        let lib_dir = prefix_in_staging.join("lib");
        if !lib_dir.exists() {
            return Err(BuildError::InstallFailed {
                message: "No lib directory found in staging".to_string(),
            }
            .into());
        }

        let mut entries = fs::read_dir(&lib_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path.file_name().unwrap().to_string_lossy();
                if dir_name.starts_with("python3.") {
                    // Extract version from directory name like "python3.11"
                    return Ok(dir_name.to_string());
                }
            }
        }

        Err(BuildError::InstallFailed {
            message: "Could not detect Python version from site-packages directory".to_string(),
        }
        .into())
    }

    /// Fix shebangs in Python scripts to use correct Python path
    async fn fix_shebangs(
        &self,
        scripts_dir: &Path,
        prefix_in_staging: &Path,
        _ctx: &BuildSystemContext,
    ) -> Result<(), Error> {
        // Detect the Python version used during build
        let python_version = self.detect_python_version(prefix_in_staging).await?;
        let target_shebang = format!(
            "#!{}/bin/{python_version}",
            sps2_config::fixed_paths::LIVE_DIR
        );

        let mut entries = fs::read_dir(scripts_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                let content = fs::read_to_string(&path).await?;
                if content.starts_with("#!") {
                    let lines: Vec<&str> = content.lines().collect();
                    if !lines.is_empty() && lines[0].contains("python") {
                        // Replace any Python shebang with our target shebang
                        let mut new_content = format!("{target_shebang}\n");
                        new_content.push_str(&lines[1..].join("\n"));
                        fs::write(&path, new_content).await?;
                    }
                }
            }
        }

        Ok(())
    }
}

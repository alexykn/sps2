//! Cargo (Rust) build system implementation

use super::{BuildSystem, BuildSystemConfig, BuildSystemContext, TestFailure, TestResults};
use async_trait::async_trait;
use sps2_errors::{BuildError, Error};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Cargo build system for Rust projects
pub struct CargoBuildSystem {
    config: BuildSystemConfig,
}

impl CargoBuildSystem {
    /// Create a new Cargo build system instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: BuildSystemConfig {
                supports_out_of_source: false, // Cargo manages its own target directory
                supports_parallel_builds: true,
                supports_incremental_builds: true,
                default_configure_args: vec![],
                default_build_args: vec!["--release".to_string()],
                env_prefix: Some("CARGO_".to_string()),
                watch_patterns: vec![
                    "Cargo.toml".to_string(),
                    "Cargo.lock".to_string(),
                    "src/**/*.rs".to_string(),
                    "build.rs".to_string(),
                ],
            },
        }
    }

    /// Setup vendored dependencies for offline builds
    async fn setup_vendoring(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        // Check if .cargo/vendor directory exists
        let vendor_dir = ctx.source_dir.join(".cargo/vendor");
        if vendor_dir.exists() {
            // Create .cargo/config.toml for vendored dependencies
            let cargo_dir = ctx.source_dir.join(".cargo");
            fs::create_dir_all(&cargo_dir).await?;

            let config_content = r#"[source.crates-io]
                                    replace-with = "vendored-sources"

                                    [source.vendored-sources]
                                    directory = "vendor"

                                    [net]
                                    offline = true
                                    "#;

            let config_path = cargo_dir.join("config.toml");
            fs::write(&config_path, config_content).await?;
        } else if !ctx.network_allowed {
            // Ensure offline mode
            return Ok(());
        }

        Ok(())
    }

    /// Get cargo build arguments
    fn get_build_args(&self, ctx: &BuildSystemContext, user_args: &[String]) -> Vec<String> {
        let mut args = vec!["build".to_string()];

        // Add default arguments
        for default_arg in &self.config.default_build_args {
            if !user_args.contains(default_arg) && !user_args.contains(&"--debug".to_string()) {
                args.push(default_arg.clone());
            }
        }

        // Add parallel jobs
        if ctx.jobs > 1 && !user_args.iter().any(|arg| arg.starts_with("-j")) {
            args.push(format!("-j{}", ctx.jobs));
        }

        // Add offline mode if network is disabled
        if !ctx.network_allowed && !user_args.contains(&"--offline".to_string()) {
            args.push("--offline".to_string());
        }

        // Add target for cross-compilation
        if let Some(cross) = &ctx.cross_compilation {
            let target = cross.host_platform.triple();
            if !user_args.iter().any(|arg| arg.starts_with("--target=")) {
                args.push(format!("--target={target}"));
            }
        }

        // Handle features
        let features = self.extract_features(user_args);
        if !features.is_empty() {
            args.push("--features".to_string());
            args.push(features.join(","));
        }

        // Add user arguments (except features which we handled above)
        args.extend(
            user_args
                .iter()
                .filter(|arg| !arg.starts_with("--features="))
                .cloned(),
        );

        args
    }

    /// Extract feature flags from arguments
    #[allow(clippy::unused_self)]
    fn extract_features(&self, args: &[String]) -> Vec<String> {
        args.iter()
            .filter_map(|arg| {
                arg.strip_prefix("--features=")
                    .map(|features| features.split(',').map(String::from).collect::<Vec<_>>())
            })
            .flatten()
            .collect()
    }

    /// Find built binaries in target directory
    async fn find_built_binaries(&self, ctx: &BuildSystemContext) -> Result<Vec<PathBuf>, Error> {
        let mut binaries = vec![];

        // Determine target directory
        let target_base = ctx.source_dir.join("target");
        let target_dir = if let Some(cross) = &ctx.cross_compilation {
            target_base
                .join(cross.host_platform.triple())
                .join("release")
        } else {
            target_base.join("release")
        };

        // Read Cargo.toml to find binary targets
        let cargo_toml = ctx.source_dir.join("Cargo.toml");
        let cargo_content = fs::read_to_string(&cargo_toml).await?;

        // Simple parsing - in production would use toml crate
        if cargo_content.contains("[[bin]]")
            || cargo_content.contains("[package]")
            || cargo_content.contains("[workspace]")
        {
            // Look for executables in target/release
            if target_dir.exists() {
                let mut entries = fs::read_dir(&target_dir).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let path = entry.path();
                    if path.is_file() {
                        // Check if it's executable
                        let metadata = fs::metadata(&path).await?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if metadata.permissions().mode() & 0o111 != 0 {
                                // Skip build artifacts
                                if let Some(name) = path.file_name() {
                                    let name_str = name.to_string_lossy();
                                    if !name_str.ends_with(".d")
                                        && !name_str.ends_with(".rlib")
                                        && !name_str.ends_with(".rmeta")
                                        && !name_str.contains("deps")
                                    {
                                        binaries.push(path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(binaries)
    }

    /// Check if this is a workspace project
    async fn is_workspace(&self, ctx: &BuildSystemContext) -> Result<bool, Error> {
        let cargo_toml = ctx.source_dir.join("Cargo.toml");
        let cargo_content = fs::read_to_string(&cargo_toml).await?;
        Ok(cargo_content.contains("[workspace]"))
    }

    /// Parse cargo test output
    #[allow(clippy::unused_self)]
    fn parse_test_output(&self, output: &str) -> (usize, usize, usize, Vec<TestFailure>) {
        let mut total = 0;
        let mut passed = 0;
        let mut failed = 0;
        let mut failures = vec![];
        let mut current_failure: Option<TestFailure> = None;

        for line in output.lines() {
            // Look for test result lines
            if line.contains("test result:") {
                // Format: "test result: ok. X passed; Y failed; Z ignored; W measured; A filtered out"
                if let Some(counts) = parse_cargo_test_summary(line) {
                    total = counts.0;
                    passed = counts.1;
                    failed = counts.2;
                }
            }
            // Capture test failures
            else if line.contains("---- ") && line.contains(" stdout ----") {
                if let Some(test_name) = line
                    .strip_prefix("---- ")
                    .and_then(|s| s.strip_suffix(" stdout ----"))
                {
                    current_failure = Some(TestFailure {
                        name: test_name.to_string(),
                        message: String::new(),
                        details: Some(String::new()),
                    });
                }
            }
            // Collect failure details
            else if let Some(failure) = &mut current_failure {
                if line == "failures:" {
                    failures.push(failure.clone());
                    current_failure = None;
                } else if let Some(details) = &mut failure.details {
                    details.push_str(line);
                    details.push('\n');
                }
            }
        }

        (total, passed, failed, failures)
    }
}

impl Default for CargoBuildSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BuildSystem for CargoBuildSystem {
    async fn detect(&self, source_dir: &Path) -> Result<bool, Error> {
        Ok(source_dir.join("Cargo.toml").exists())
    }

    fn get_config_options(&self) -> BuildSystemConfig {
        self.config.clone()
    }

    async fn configure(&self, ctx: &BuildSystemContext, _args: &[String]) -> Result<(), Error> {
        // Cargo doesn't have a separate configure step
        // But we can set up vendoring and check dependencies

        // Setup vendoring if needed
        self.setup_vendoring(ctx).await?;

        // Verify cargo is available
        let result = ctx.execute("cargo", &["--version"], None).await?;
        if !result.success {
            return Err(BuildError::ConfigureFailed {
                message: "cargo not found in PATH".to_string(),
            }
            .into());
        }

        // Check if we can read Cargo.toml
        let cargo_toml = ctx.source_dir.join("Cargo.toml");
        if !cargo_toml.exists() {
            return Err(BuildError::ConfigureFailed {
                message: "Cargo.toml not found".to_string(),
            }
            .into());
        }

        Ok(())
    }

    async fn build(&self, ctx: &BuildSystemContext, args: &[String]) -> Result<(), Error> {
        let build_args = self.get_build_args(ctx, args);
        let arg_refs: Vec<&str> = build_args.iter().map(String::as_str).collect();

        // Run cargo build
        let result = ctx
            .execute("cargo", &arg_refs, Some(&ctx.source_dir))
            .await?;

        if !result.success {
            return Err(BuildError::CompilationFailed {
                message: format!("cargo build failed: {}", result.stderr),
            }
            .into());
        }

        Ok(())
    }

    async fn test(&self, ctx: &BuildSystemContext) -> Result<TestResults, Error> {
        let start = std::time::Instant::now();

        let mut test_args = vec!["test"];

        // Add release flag if we built in release mode
        if let Ok(extra_env) = ctx.extra_env.read() {
            if extra_env.get("PROFILE").map(String::as_str) == Some("release") {
                test_args.push("--release");
            }
        }

        // Add offline mode if needed
        if !ctx.network_allowed {
            test_args.push("--offline");
        }

        // Add parallel jobs
        let jobs_str;
        if ctx.jobs > 1 {
            jobs_str = ctx.jobs.to_string();
            test_args.push("--");
            test_args.push("--test-threads");
            test_args.push(&jobs_str);
        }

        // Run cargo test
        let result = ctx
            .execute("cargo", &test_args, Some(&ctx.source_dir))
            .await?;

        let duration = start.elapsed().as_secs_f64();
        let output = format!("{}\n{}", result.stdout, result.stderr);

        // Parse test results
        let (total, passed, failed, failures) = self.parse_test_output(&output);

        Ok(TestResults {
            total,
            passed,
            failed,
            skipped: total.saturating_sub(passed + failed),
            duration,
            output,
            failures,
        })
    }

    async fn install(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        // Find built binaries
        let binaries = self.find_built_binaries(ctx).await?;

        if binaries.is_empty() {
            // Check if this is a workspace project
            if self.is_workspace(ctx).await? {
                return Err(BuildError::InstallFailed {
                    message: "No binaries found for workspace project. The build may have failed or the workspace may not contain any binary targets.".to_string(),
                }
                .into());
            }

            // Try cargo install as fallback for single-crate projects
            // Since cargo install --root expects an actual directory path, we need to install
            // to a temp location and then move files to the staging dir with BUILD_PREFIX structure
            let temp_install_dir = ctx.build_dir.join("cargo_install_temp");
            fs::create_dir_all(&temp_install_dir).await?;

            let temp_install_str = temp_install_dir.display().to_string();
            let install_args = vec![
                "install",
                "--path",
                ".",
                "--root",
                &temp_install_str,
                "--offline",
            ];

            let result = ctx
                .execute("cargo", &install_args, Some(&ctx.source_dir))
                .await?;

            if !result.success {
                return Err(BuildError::InstallFailed {
                    message: format!("cargo install failed: {}", result.stderr),
                }
                .into());
            }

            // Move files from temp install to staging with BUILD_PREFIX structure
            let staging_dir = ctx.env.staging_dir();
            let prefix_path = staging_dir.join(ctx.env.get_build_prefix().trim_start_matches('/'));

            // Move bin directory
            let temp_bin = temp_install_dir.join("bin");
            if temp_bin.exists() {
                let dest_bin = prefix_path.join("bin");
                fs::create_dir_all(&dest_bin).await?;

                let mut entries = fs::read_dir(&temp_bin).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let src = entry.path();
                    if let Some(filename) = src.file_name() {
                        let dest = dest_bin.join(filename);
                        fs::rename(&src, &dest).await?;
                    }
                }
            }

            // Clean up temp directory
            let _ = fs::remove_dir_all(&temp_install_dir).await;
        } else {
            // Manually copy binaries to staging with BUILD_PREFIX structure
            let staging_dir = ctx.env.staging_dir();
            let prefix_path = staging_dir.join(ctx.env.get_build_prefix().trim_start_matches('/'));
            let staging_bin = prefix_path.join("bin");
            fs::create_dir_all(&staging_bin).await?;

            for binary in binaries {
                if let Some(filename) = binary.file_name() {
                    let dest = staging_bin.join(filename);
                    fs::copy(&binary, &dest).await?;

                    // Preserve executable permissions
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let metadata = fs::metadata(&binary).await?;
                        let mut perms = metadata.permissions();
                        perms.set_mode(0o755);
                        fs::set_permissions(&dest, perms).await?;
                    }
                }
            }
        }

        Ok(())
    }

    fn get_env_vars(&self, ctx: &BuildSystemContext) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        // Cargo-specific environment variables
        vars.insert(
            "CARGO_TARGET_DIR".to_string(),
            ctx.build_dir.join("target").display().to_string(),
        );

        // Set CARGO_HOME to deps directory for isolation
        vars.insert(
            "CARGO_HOME".to_string(),
            ctx.env.deps_prefix().join(".cargo").display().to_string(),
        );

        // Enable colored output
        vars.insert("CARGO_TERM_COLOR".to_string(), "always".to_string());

        // Set profile
        vars.insert("PROFILE".to_string(), "release".to_string());

        // Compiler cache support
        if let Some(cache_config) = &ctx.cache_config {
            if cache_config.use_compiler_cache {
                if let super::core::CompilerCacheType::SCCache = &cache_config.compiler_cache_type {
                    vars.insert("RUSTC_WRAPPER".to_string(), "sccache".to_string());
                }
            }
        }

        // Cross-compilation support
        if let Some(cross) = &ctx.cross_compilation {
            let target_triple = cross.host_platform.triple();
            let target_upper = target_triple.replace('-', "_").to_uppercase();

            // Set linker for target
            vars.insert(
                format!("CARGO_TARGET_{target_upper}_LINKER"),
                cross.toolchain.cc.clone(),
            );

            // Set other target-specific variables
            vars.insert(
                format!("CARGO_TARGET_{target_upper}_RUSTFLAGS"),
                format!("-C link-arg=--sysroot={}", cross.sysroot.display()),
            );
        }

        vars
    }

    fn name(&self) -> &'static str {
        "cargo"
    }

    fn prefers_out_of_source_build(&self) -> bool {
        // Cargo manages its own target directory
        false
    }
}

/// Parse cargo test summary line
fn parse_cargo_test_summary(line: &str) -> Option<(usize, usize, usize)> {
    // Format: "test result: ok. X passed; Y failed; Z ignored; W measured; A filtered out"
    // Extract the part after "ok." or "FAILED."
    let stats_part = if let Some(pos) = line.find(". ") {
        &line[pos + 2..]
    } else {
        line
    };

    let mut passed = 0;
    let mut failed = 0;
    let mut ignored = 0;

    for part in stats_part.split(';') {
        let part = part.trim();
        if let Some((num_str, rest)) = part.split_once(' ') {
            if let Ok(num) = num_str.parse::<usize>() {
                if rest.starts_with("passed") {
                    passed = num;
                } else if rest.starts_with("failed") {
                    failed = num;
                } else if rest.starts_with("ignored") {
                    ignored = num;
                }
            }
        }
    }

    if passed > 0 || failed > 0 || ignored > 0 {
        let total = passed + failed + ignored;
        Some((total, passed, failed))
    } else {
        None
    }
}

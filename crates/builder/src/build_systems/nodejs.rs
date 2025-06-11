//! Node.js build system implementation

use super::{BuildSystem, BuildSystemConfig, BuildSystemContext, TestFailure, TestResults};
use async_trait::async_trait;
use sps2_errors::{BuildError, Error};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

/// Node.js build system supporting npm, yarn, and pnpm
pub struct NodeJsBuildSystem {
    config: BuildSystemConfig,
}

impl NodeJsBuildSystem {
    /// Create a new Node.js build system instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: BuildSystemConfig {
                supports_out_of_source: false, // Node.js builds in-place
                supports_parallel_builds: true,
                supports_incremental_builds: true,
                default_configure_args: vec![],
                default_build_args: vec![],
                env_prefix: Some("NODE_".to_string()),
                watch_patterns: vec![
                    "package.json".to_string(),
                    "package-lock.json".to_string(),
                    "yarn.lock".to_string(),
                    "pnpm-lock.yaml".to_string(),
                    "**/*.js".to_string(),
                    "**/*.ts".to_string(),
                    "**/*.jsx".to_string(),
                    "**/*.tsx".to_string(),
                ],
            },
        }
    }

    /// Detect which package manager to use
    async fn detect_package_manager(&self, source_dir: &Path) -> Result<PackageManager, Error> {
        // Check for lock files first (most reliable)
        if source_dir.join("pnpm-lock.yaml").exists() {
            return Ok(PackageManager::Pnpm);
        }
        if source_dir.join("yarn.lock").exists() {
            return Ok(PackageManager::Yarn);
        }
        if source_dir.join("package-lock.json").exists() {
            return Ok(PackageManager::Npm);
        }

        // Check for packageManager field in package.json
        let package_json = source_dir.join("package.json");
        if package_json.exists() {
            let content = fs::read_to_string(&package_json).await?;
            if content.contains("\"packageManager\"") {
                if content.contains("pnpm@") {
                    return Ok(PackageManager::Pnpm);
                } else if content.contains("yarn@") {
                    return Ok(PackageManager::Yarn);
                }
            }
        }

        // Default to npm
        Ok(PackageManager::Npm)
    }

    /// Setup offline mode and vendoring
    async fn setup_offline_mode(
        &self,
        ctx: &BuildSystemContext,
        pm: &PackageManager,
    ) -> Result<(), Error> {
        if !ctx.network_allowed {
            match pm {
                PackageManager::Npm => {
                    // Create .npmrc for offline mode
                    let npmrc_content = "offline=true\n";
                    fs::write(ctx.source_dir.join(".npmrc"), npmrc_content).await?;
                }
                PackageManager::Yarn => {
                    // Yarn offline mode is handled via command line
                }
                PackageManager::Pnpm => {
                    // Create .pnpmfile.cjs for offline mode
                    let pnpmrc_content = "offline=true\n";
                    fs::write(ctx.source_dir.join(".pnpmrc"), pnpmrc_content).await?;
                }
            }
        }

        // Setup vendoring if node_modules exists
        let node_modules = ctx.source_dir.join("node_modules");
        if !node_modules.exists() && ctx.source_dir.join("vendor").exists() {
            // Link vendor to node_modules
            #[cfg(unix)]
            {
                use std::os::unix::fs::symlink;
                symlink(ctx.source_dir.join("vendor"), &node_modules)?;
            }
        }

        Ok(())
    }

    /// Get install command for package manager
    fn get_install_command(
        &self,
        pm: &PackageManager,
        offline: bool,
        has_lock_file: bool,
    ) -> Vec<String> {
        match pm {
            PackageManager::Npm => {
                // Use ci only if lock file exists, otherwise use install
                let mut args = if has_lock_file {
                    vec!["ci".to_string()]
                } else {
                    vec!["install".to_string()]
                };
                if offline {
                    args.push("--offline".to_string());
                }
                args.push("--no-audit".to_string());
                args.push("--no-fund".to_string());
                args
            }
            PackageManager::Yarn => {
                let mut args = vec!["install".to_string()];
                if offline {
                    args.push("--offline".to_string());
                }
                args.push("--frozen-lockfile".to_string());
                args.push("--non-interactive".to_string());
                args
            }
            PackageManager::Pnpm => {
                let mut args = vec!["install".to_string()];
                if offline {
                    args.push("--offline".to_string());
                }
                args.push("--frozen-lockfile".to_string());
                args
            }
        }
    }

    /// Get build script name from package.json
    async fn get_build_script(&self, source_dir: &Path) -> Result<Option<String>, Error> {
        let package_json = source_dir.join("package.json");
        if !package_json.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&package_json).await?;

        // Simple parsing - look for build script
        if content.contains("\"build\":") {
            return Ok(Some("build".to_string()));
        }
        if content.contains("\"compile\":") {
            return Ok(Some("compile".to_string()));
        }
        if content.contains("\"dist\":") {
            return Ok(Some("dist".to_string()));
        }

        Ok(None)
    }

    /// Parse test output from various test runners
    fn parse_test_output(&self, output: &str) -> (usize, usize, usize, Vec<TestFailure>) {
        let mut total = 0;
        let mut passed = 0;
        let mut failed = 0;
        let mut failures = vec![];

        // Jest pattern: "Tests:       1 failed, 2 passed, 3 total"
        if output.contains("Tests:") {
            for line in output.lines() {
                if line.contains("Tests:") && line.contains("total") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    for (i, part) in parts.iter().enumerate() {
                        if let Ok(num) = part.parse::<usize>() {
                            if i + 1 < parts.len() {
                                if let Some(next_part) = parts.get(i + 1) {
                                    match *next_part {
                                        "passed" | "passed," => passed = num,
                                        "failed" | "failed," => failed = num,
                                        "total" => total = num,
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Mocha pattern: "  2 passing"
        else if output.contains("passing") {
            for line in output.lines() {
                if line.contains("passing") {
                    if let Some(num_str) = line.split_whitespace().next() {
                        if let Ok(num) = num_str.parse::<usize>() {
                            passed = num;
                            total = num;
                        }
                    }
                }
                if line.contains("failing") {
                    if let Some(num_str) = line.split_whitespace().next() {
                        if let Ok(num) = num_str.parse::<usize>() {
                            failed = num;
                            total += num;
                        }
                    }
                }
            }
        }
        // TAP format: "ok 1 - test description"
        else if output.contains("TAP version") || output.contains("ok 1") {
            for line in output.lines() {
                if line.starts_with("ok ") || line.starts_with("not ok ") {
                    total += 1;
                    if line.starts_with("ok ") {
                        passed += 1;
                    } else {
                        failed += 1;
                        if let Some(desc) = line.split(" - ").nth(1) {
                            failures.push(TestFailure {
                                name: desc.to_string(),
                                message: line.to_string(),
                                details: None,
                            });
                        }
                    }
                }
            }
        }

        // If no pattern matched but tests ran
        if total == 0 && (output.contains("test") || output.contains("spec")) {
            if output.contains("failed") || output.contains("error") {
                total = 1;
                failed = 1;
            } else {
                total = 1;
                passed = 1;
            }
        }

        (total, passed, failed, failures)
    }

    /// Find and copy built artifacts
    async fn copy_built_artifacts(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        let staging_dir = ctx.env.staging_dir();
        let prefix_path = staging_dir.join(ctx.env.get_build_prefix().trim_start_matches('/'));

        // Common output directories
        let possible_dirs = vec!["dist", "build", "out", "lib"];

        for dir_name in possible_dirs {
            let output_dir = ctx.source_dir.join(dir_name);
            if output_dir.exists() {
                // Copy to staging with BUILD_PREFIX structure
                let dest = prefix_path.join(dir_name);
                fs::create_dir_all(&dest).await?;
                copy_dir_recursive(&output_dir, &dest).await?;
            }
        }

        // Handle bin entries from package.json
        let package_json_path = ctx.source_dir.join("package.json");
        if package_json_path.exists() {
            let content = fs::read_to_string(&package_json_path).await?;
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(bin) = json.get("bin") {
                    let dest_bin = prefix_path.join("bin");
                    fs::create_dir_all(&dest_bin).await?;

                    match bin {
                        serde_json::Value::String(script) => {
                            // Single bin entry
                            let script_path = ctx.source_dir.join(script);
                            if script_path.exists() {
                                let bin_name =
                                    json.get("name").and_then(|n| n.as_str()).unwrap_or("bin");
                                let dest = dest_bin.join(bin_name);
                                fs::copy(&script_path, &dest).await?;
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    let mut perms = fs::metadata(&dest).await?.permissions();
                                    perms.set_mode(0o755);
                                    fs::set_permissions(&dest, perms).await?;
                                }
                            }
                        }
                        serde_json::Value::Object(bins) => {
                            // Multiple bin entries
                            for (name, script) in bins {
                                if let Some(script_str) = script.as_str() {
                                    let script_path = ctx.source_dir.join(script_str);
                                    if script_path.exists() {
                                        let dest = dest_bin.join(name);
                                        fs::copy(&script_path, &dest).await?;
                                        #[cfg(unix)]
                                        {
                                            use std::os::unix::fs::PermissionsExt;
                                            let mut perms =
                                                fs::metadata(&dest).await?.permissions();
                                            perms.set_mode(0o755);
                                            fs::set_permissions(&dest, perms).await?;
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Copy binaries from node_modules/.bin if they exist
        let bin_dir = ctx.source_dir.join("node_modules/.bin");
        if bin_dir.exists() {
            let dest_bin = prefix_path.join("bin");
            fs::create_dir_all(&dest_bin).await?;

            let mut entries = fs::read_dir(&bin_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_file() {
                    let filename = path.file_name().unwrap();
                    let dest = dest_bin.join(filename);
                    fs::copy(&path, &dest).await?;

                    // Make executable
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mut perms = fs::metadata(&dest).await?.permissions();
                        perms.set_mode(0o755);
                        fs::set_permissions(&dest, perms).await?;
                    }
                }
            }
        }

        Ok(())
    }
}

/// Node.js package managers
#[derive(Debug, Clone)]
enum PackageManager {
    Npm,
    Yarn,
    Pnpm,
}

impl PackageManager {
    fn command(&self) -> &str {
        match self {
            Self::Npm => "npm",
            Self::Yarn => "yarn",
            Self::Pnpm => "pnpm",
        }
    }
}

impl Default for NodeJsBuildSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BuildSystem for NodeJsBuildSystem {
    async fn detect(&self, source_dir: &Path) -> Result<bool, Error> {
        Ok(source_dir.join("package.json").exists())
    }

    fn get_config_options(&self) -> BuildSystemConfig {
        self.config.clone()
    }

    async fn configure(&self, ctx: &BuildSystemContext, _args: &[String]) -> Result<(), Error> {
        // Detect package manager
        let pm = self.detect_package_manager(&ctx.source_dir).await?;

        // Verify package manager is available
        let result = ctx.execute(pm.command(), &["--version"], None).await?;

        if !result.success {
            return Err(BuildError::ConfigureFailed {
                message: format!("{} not found in PATH", pm.command()),
            }
            .into());
        }

        // Setup offline mode if needed
        self.setup_offline_mode(ctx, &pm).await?;

        // Store package manager for later use
        if let Ok(mut extra_env) = ctx.extra_env.write() {
            extra_env.insert("NODE_PACKAGE_MANAGER".to_string(), format!("{:?}", pm));
        }

        Ok(())
    }

    async fn build(&self, ctx: &BuildSystemContext, args: &[String]) -> Result<(), Error> {
        // Get package manager from configure phase
        let pm_str = if let Ok(extra_env) = ctx.extra_env.read() {
            extra_env
                .get("NODE_PACKAGE_MANAGER")
                .cloned()
                .ok_or_else(|| BuildError::ConfigureFailed {
                    message: "Package manager not detected".to_string(),
                })?
        } else {
            return Err(BuildError::ConfigureFailed {
                message: "Cannot access extra environment".to_string(),
            }
            .into());
        };

        let pm = match pm_str.as_str() {
            "Npm" => PackageManager::Npm,
            "Yarn" => PackageManager::Yarn,
            "Pnpm" => PackageManager::Pnpm,
            _ => PackageManager::Npm,
        };

        // Check if lock file exists
        let has_lock_file = match &pm {
            PackageManager::Npm => ctx.source_dir.join("package-lock.json").exists(),
            PackageManager::Yarn => ctx.source_dir.join("yarn.lock").exists(),
            PackageManager::Pnpm => ctx.source_dir.join("pnpm-lock.yaml").exists(),
        };

        // Check if package.json has any dependencies
        let package_json = ctx.source_dir.join("package.json");
        let has_dependencies = if package_json.exists() {
            let content = fs::read_to_string(&package_json).await?;
            content.contains("\"dependencies\"") || content.contains("\"devDependencies\"")
        } else {
            false
        };

        // Only run install if there are dependencies or a lock file
        if has_dependencies || has_lock_file {
            let install_args = self.get_install_command(&pm, !ctx.network_allowed, has_lock_file);
            let arg_refs: Vec<&str> = install_args.iter().map(String::as_str).collect();

            let result = ctx
                .execute(pm.command(), &arg_refs, Some(&ctx.source_dir))
                .await?;

            if !result.success {
                return Err(BuildError::CompilationFailed {
                    message: format!("{} install failed: {}", pm.command(), result.stderr),
                }
                .into());
            }
        }

        // Run build script if it exists
        if let Some(build_script) = self.get_build_script(&ctx.source_dir).await? {
            let mut run_args = vec!["run", &build_script];

            // Add user arguments
            if !args.is_empty() {
                run_args.push("--");
                run_args.extend(args.iter().map(String::as_str));
            }

            let result = ctx
                .execute(pm.command(), &run_args, Some(&ctx.source_dir))
                .await?;

            if !result.success {
                return Err(BuildError::CompilationFailed {
                    message: format!("Build script failed: {}", result.stderr),
                }
                .into());
            }
        }

        Ok(())
    }

    async fn test(&self, ctx: &BuildSystemContext) -> Result<TestResults, Error> {
        let start = std::time::Instant::now();

        // Get package manager
        let pm_str = if let Ok(extra_env) = ctx.extra_env.read() {
            extra_env
                .get("NODE_PACKAGE_MANAGER")
                .cloned()
                .unwrap_or_else(|| "Npm".to_string())
        } else {
            "Npm".to_string()
        };

        let pm = match pm_str.as_str() {
            "Npm" => PackageManager::Npm,
            "Yarn" => PackageManager::Yarn,
            "Pnpm" => PackageManager::Pnpm,
            _ => PackageManager::Npm,
        };

        // Check if test script exists
        let package_json = ctx.source_dir.join("package.json");
        let has_test_script = if package_json.exists() {
            let content = fs::read_to_string(&package_json).await?;
            content.contains("\"test\":")
        } else {
            false
        };

        if !has_test_script {
            // No tests defined
            return Ok(TestResults {
                total: 0,
                passed: 0,
                failed: 0,
                skipped: 0,
                duration: 0.0,
                output: "No test script defined in package.json".to_string(),
                failures: vec![],
            });
        }

        // Run tests
        let result = ctx
            .execute(pm.command(), &["test"], Some(&ctx.source_dir))
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
        // Copy built artifacts to staging
        self.copy_built_artifacts(ctx).await?;

        // Also copy package.json for metadata with BUILD_PREFIX structure
        let package_json_src = ctx.source_dir.join("package.json");
        if package_json_src.exists() {
            let staging_dir = ctx.env.staging_dir();
            let prefix_path = staging_dir.join(ctx.env.get_build_prefix().trim_start_matches('/'));
            let package_json_dest = prefix_path.join("package.json");
            fs::copy(&package_json_src, &package_json_dest).await?;
        }

        Ok(())
    }

    fn get_env_vars(&self, ctx: &BuildSystemContext) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        // Set NODE_ENV to production for builds
        vars.insert("NODE_ENV".to_string(), "production".to_string());

        // Disable telemetry and update checks
        vars.insert("DISABLE_OPENCOLLECTIVE".to_string(), "1".to_string());
        vars.insert("ADBLOCK".to_string(), "1".to_string());
        vars.insert("DISABLE_TELEMETRY".to_string(), "1".to_string());
        vars.insert("NO_UPDATE_NOTIFIER".to_string(), "1".to_string());

        // Set npm configuration
        vars.insert("NPM_CONFIG_LOGLEVEL".to_string(), "warn".to_string());
        vars.insert("NPM_CONFIG_FUND".to_string(), "false".to_string());
        vars.insert("NPM_CONFIG_AUDIT".to_string(), "false".to_string());

        // Set cache directories
        if let Some(cache_config) = &ctx.cache_config {
            vars.insert(
                "NPM_CONFIG_CACHE".to_string(),
                cache_config.cache_dir.join("npm").display().to_string(),
            );
            vars.insert(
                "YARN_CACHE_FOLDER".to_string(),
                cache_config.cache_dir.join("yarn").display().to_string(),
            );
            vars.insert(
                "PNPM_HOME".to_string(),
                cache_config.cache_dir.join("pnpm").display().to_string(),
            );
        }

        vars
    }

    fn name(&self) -> &'static str {
        "nodejs"
    }
}

/// Recursively copy directory contents
async fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), Error> {
    fs::create_dir_all(dst).await?;

    let mut entries = fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            Box::pin(copy_dir_recursive(&src_path, &dst_path)).await?;
        } else {
            fs::copy(&src_path, &dst_path).await?;
        }
    }

    Ok(())
}

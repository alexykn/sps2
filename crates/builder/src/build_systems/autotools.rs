//! GNU Autotools build system implementation

use super::{BuildSystem, BuildSystemConfig, BuildSystemContext, TestResults};
use async_trait::async_trait;
use sps2_errors::{BuildError, Error};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

/// GNU Autotools build system
pub struct AutotoolsBuildSystem {
    config: BuildSystemConfig,
}

impl AutotoolsBuildSystem {
    /// Create a new Autotools build system instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: BuildSystemConfig {
                supports_out_of_source: true,
                supports_parallel_builds: true,
                supports_incremental_builds: true,
                default_configure_args: vec![],
                default_build_args: vec![],
                env_prefix: None,
                watch_patterns: vec![
                    "configure".to_string(),
                    "configure.ac".to_string(),
                    "configure.in".to_string(),
                    "Makefile.am".to_string(),
                    "Makefile.in".to_string(),
                ],
            },
        }
    }

    /// Check if autoreconf is needed
    async fn needs_autoreconf(&self, source_dir: &Path) -> Result<bool, Error> {
        // If configure exists and is newer than configure.ac, no need for autoreconf
        let configure_path = source_dir.join("configure");
        if !configure_path.exists() {
            return Ok(true);
        }

        // Check for configure.ac or configure.in
        let configure_ac = source_dir.join("configure.ac");
        let configure_in = source_dir.join("configure.in");

        if configure_ac.exists() {
            let configure_meta = fs::metadata(&configure_path).await?;
            let configure_ac_meta = fs::metadata(&configure_ac).await?;

            // If configure.ac is newer than configure, run autoreconf
            if let (Ok(configure_time), Ok(ac_time)) =
                (configure_meta.modified(), configure_ac_meta.modified())
            {
                return Ok(ac_time > configure_time);
            }
        }

        Ok(configure_ac.exists() || configure_in.exists())
    }

    /// Run autoreconf if needed
    async fn run_autoreconf(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        if self.needs_autoreconf(&ctx.source_dir).await? {
            let result = ctx
                .execute("autoreconf", &["-fiv"], Some(&ctx.source_dir))
                .await?;

            if !result.success {
                return Err(BuildError::ConfigureFailed {
                    message: format!("autoreconf failed: {}", result.stderr),
                }
                .into());
            }
        }
        Ok(())
    }

    /// Handle config.cache for faster reconfiguration
    async fn handle_config_cache(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        if let Some(cache_config) = &ctx.cache_config {
            let cache_file = cache_config.cache_dir.join("config.cache");
            if cache_file.exists() {
                // Copy config.cache to build directory
                let dest = ctx.build_dir.join("config.cache");
                fs::copy(&cache_file, &dest).await?;
            }
        }
        Ok(())
    }

    /// Get configure arguments including cross-compilation
    #[allow(clippy::unused_self)]
    fn get_configure_args(&self, ctx: &BuildSystemContext, user_args: &[String]) -> Vec<String> {
        let mut args = vec![];

        // Add prefix - use LIVE_PREFIX for runtime installation location
        if !user_args.iter().any(|arg| arg.starts_with("--prefix=")) {
            args.push(format!("--prefix={}", ctx.env.get_live_prefix()));
        }

        // Handle cross-compilation
        if let Some(cross) = &ctx.cross_compilation {
            args.push(format!("--host={}", cross.host_platform.triple()));
            args.push(format!("--build={}", cross.build_platform.triple()));
            if let Some(target) = &cross.target_platform {
                args.push(format!("--target={}", target.triple()));
            }
        }

        // Add user arguments
        args.extend(user_args.iter().cloned());

        // Add compiler flags from environment
        if let Some(cflags) = ctx.get_all_env_vars().get("CFLAGS") {
            args.push(format!("CFLAGS={cflags}"));
        }
        if let Some(cxxflags) = ctx.get_all_env_vars().get("CXXFLAGS") {
            args.push(format!("CXXFLAGS={cxxflags}"));
        }

        // Handle LDFLAGS with RPATH for macOS
        let mut ldflags = ctx
            .get_all_env_vars()
            .get("LDFLAGS")
            .cloned()
            .unwrap_or_default();

        if cfg!(target_os = "macos") {
            // Add RPATH to the library directory for runtime linking
            let rpath_flag = format!("-Wl,-rpath,{}/lib", ctx.env.get_live_prefix());
            if !ldflags.contains(&rpath_flag) {
                if !ldflags.is_empty() {
                    ldflags.push(' ');
                }
                ldflags.push_str(&rpath_flag);
            }
        }

        if !ldflags.is_empty() {
            args.push(format!("LDFLAGS={ldflags}"));
        }

        args
    }
}

impl Default for AutotoolsBuildSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BuildSystem for AutotoolsBuildSystem {
    async fn detect(&self, source_dir: &Path) -> Result<bool, Error> {
        // Check for configure script
        if source_dir.join("configure").exists() {
            return Ok(true);
        }

        // Check for configure.ac or configure.in (needs autoreconf)
        if source_dir.join("configure.ac").exists() || source_dir.join("configure.in").exists() {
            return Ok(true);
        }

        // Check for Makefile.am (automake project)
        if source_dir.join("Makefile.am").exists() {
            return Ok(true);
        }

        Ok(false)
    }

    fn get_config_options(&self) -> BuildSystemConfig {
        self.config.clone()
    }

    async fn configure(&self, ctx: &BuildSystemContext, args: &[String]) -> Result<(), Error> {
        // Run autoreconf if needed
        self.run_autoreconf(ctx).await?;

        // Handle config cache
        self.handle_config_cache(ctx).await?;

        // Create build directory if out-of-source build
        if ctx.source_dir != ctx.build_dir {
            fs::create_dir_all(&ctx.build_dir).await?;
        }

        // Get configure script path
        let configure_path = if ctx.source_dir == ctx.build_dir {
            "./configure".to_string()
        } else {
            ctx.source_dir.join("configure").display().to_string()
        };

        // Build configure command
        let configure_args = self.get_configure_args(ctx, args);
        let mut cmd_args = vec![configure_path];
        cmd_args.extend(configure_args);

        // Run configure
        let result = ctx
            .execute("sh", &["-c", &cmd_args.join(" ")], Some(&ctx.build_dir))
            .await?;

        if !result.success {
            return Err(BuildError::ConfigureFailed {
                message: format!("configure failed: {}", result.stderr),
            }
            .into());
        }

        // Save config.cache if caching is enabled
        if let Some(cache_config) = &ctx.cache_config {
            let config_cache = ctx.build_dir.join("config.cache");
            if config_cache.exists() {
                let dest = cache_config.cache_dir.join("config.cache");
                fs::create_dir_all(&cache_config.cache_dir).await?;
                fs::copy(&config_cache, &dest).await?;
            }
        }

        Ok(())
    }

    async fn build(&self, ctx: &BuildSystemContext, args: &[String]) -> Result<(), Error> {
        let mut make_args = vec![];

        // Add parallel jobs
        if ctx.jobs > 1 {
            make_args.push(format!("-j{}", ctx.jobs));
        }

        // Add user arguments
        make_args.extend(args.iter().cloned());

        // Convert to string slices
        let arg_refs: Vec<&str> = make_args.iter().map(String::as_str).collect();

        // Run make
        let result = ctx.execute("make", &arg_refs, Some(&ctx.build_dir)).await?;

        if !result.success {
            return Err(BuildError::CompilationFailed {
                message: format!("make failed: {}", result.stderr),
            }
            .into());
        }

        Ok(())
    }

    async fn test(&self, ctx: &BuildSystemContext) -> Result<TestResults, Error> {
        // Run make check or make test
        let start = std::time::Instant::now();

        // Try "make check" first (more common in autotools)
        let result = ctx
            .execute("make", &["check"], Some(&ctx.build_dir))
            .await?;

        let success = if result.success {
            true
        } else {
            // Fallback to "make test"
            let test_result = ctx.execute("make", &["test"], Some(&ctx.build_dir)).await?;
            test_result.success
        };

        let duration = start.elapsed().as_secs_f64();

        // Parse test results from output
        // This is a simple implementation; real implementation would parse test suite output
        let output = format!("{}\n{}", result.stdout, result.stderr);
        let (total, passed, failed, skipped) = if success {
            // If make check succeeded, assume all tests passed
            // Real implementation would parse TESTS output
            (1, 1, 0, 0)
        } else {
            (1, 0, 1, 0)
        };

        Ok(TestResults {
            total,
            passed,
            failed,
            skipped,
            duration,
            output,
            failures: vec![],
        })
    }

    async fn install(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        // Run make install with DESTDIR
        let result = ctx
            .execute(
                "make",
                &[
                    "install",
                    &format!("DESTDIR={}", ctx.env.staging_dir().display()),
                ],
                Some(&ctx.build_dir),
            )
            .await?;

        if !result.success {
            return Err(BuildError::InstallFailed {
                message: format!("make install failed: {}", result.stderr),
            }
            .into());
        }

        // No need to adjust staged files since we're using BUILD_PREFIX now
        // which already includes the package-name-version structure
        Ok(())
    }

    fn get_env_vars(&self, ctx: &BuildSystemContext) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        // Standard autotools environment variables
        vars.insert("PREFIX".to_string(), ctx.prefix.display().to_string());
        vars.insert(
            "DESTDIR".to_string(),
            ctx.env.staging_dir().display().to_string(),
        );

        // Compiler cache setup
        if let Some(cache_config) = &ctx.cache_config {
            if cache_config.use_compiler_cache {
                match cache_config.compiler_cache_type {
                    super::core::CompilerCacheType::CCache => {
                        vars.insert("CC".to_string(), "ccache gcc".to_string());
                        vars.insert("CXX".to_string(), "ccache g++".to_string());
                    }
                    super::core::CompilerCacheType::SCCache => {
                        vars.insert("RUSTC_WRAPPER".to_string(), "sccache".to_string());
                    }
                    super::core::CompilerCacheType::DistCC => {}
                }
            }
        }

        vars
    }

    fn name(&self) -> &'static str {
        "autotools"
    }

    fn prefers_out_of_source_build(&self) -> bool {
        // Autotools supports both, but in-source is traditional
        false
    }
}

impl AutotoolsBuildSystem {
    /// Adjust staged files by moving them from stage/PREFIX/* to stage/*
    async fn adjust_staged_files(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        use tokio::fs;

        let staging_dir = ctx.env.staging_dir();
        let prefix_in_staging =
            staging_dir.join(ctx.prefix.strip_prefix("/").unwrap_or(&ctx.prefix));

        // Debug: log what we're checking
        if std::env::var("DEBUG").is_ok() {
            eprintln!(
                "Autotools: Checking for files in {} to move to {}",
                prefix_in_staging.display(),
                staging_dir.display()
            );
        }

        // If the full prefix path exists in staging, we need to move its contents up
        if prefix_in_staging.exists() && prefix_in_staging != staging_dir {
            // Debug: found prefix directory
            if std::env::var("DEBUG").is_ok() {
                eprintln!("Autotools: Found prefix directory, adjusting staged files");
            }

            // Use a non-recursive approach to move files
            self.move_directory_contents(&prefix_in_staging, staging_dir)
                .await?;

            // Clean up the now-empty prefix directories
            self.cleanup_empty_dirs(&prefix_in_staging).await?;

            // Also clean up any parent directories that may now be empty
            // For example, if we moved from stage/opt/pm/live/*, we need to clean up
            // stage/opt/pm/live, stage/opt/pm, and stage/opt if they're empty
            let mut parent = prefix_in_staging.parent();
            while let Some(p) = parent {
                if p == staging_dir {
                    break; // Don't try to remove the staging dir itself
                }
                if self.is_directory_empty(p).await? {
                    if std::env::var("DEBUG").is_ok() {
                        eprintln!("Autotools: Removing empty directory: {}", p.display());
                    }
                    fs::remove_dir(p).await?;
                } else {
                    break; // Stop if we hit a non-empty directory
                }
                parent = p.parent();
            }
        }

        Ok(())
    }

    /// Move all contents from source directory to destination
    #[allow(clippy::only_used_in_recursion)]
    fn move_directory_contents<'a>(
        &'a self,
        source: &'a Path,
        dest: &'a Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            use tokio::fs;

            let mut entries = fs::read_dir(source).await?;

            while let Some(entry) = entries.next_entry().await? {
                let source_path = entry.path();
                let file_name = source_path
                    .file_name()
                    .ok_or_else(|| Error::internal("Invalid file name"))?;
                let dest_path = dest.join(file_name);

                // Create parent directory if needed
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent).await?;
                }

                // If it's a directory, move its contents recursively
                if source_path.is_dir() {
                    fs::create_dir_all(&dest_path).await?;
                    self.move_directory_contents(&source_path, &dest_path)
                        .await?;
                    // Remove the now-empty source directory
                    fs::remove_dir(&source_path).await?;
                } else {
                    // Move the file
                    fs::rename(&source_path, &dest_path)
                        .await
                        .map_err(|e| Error::internal(format!("Failed to move file: {}", e)))?;
                }
            }

            Ok(())
        })
    }

    /// Check if a directory is empty
    fn is_directory_empty<'a>(
        &'a self,
        path: &'a Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, Error>> + Send + 'a>> {
        Box::pin(async move {
            use tokio::fs;

            if !path.is_dir() {
                return Ok(false);
            }

            let mut entries = fs::read_dir(path).await?;
            Ok(entries.next_entry().await?.is_none())
        })
    }

    /// Recursively remove empty directories
    #[allow(clippy::only_used_in_recursion)]
    fn cleanup_empty_dirs<'a>(
        &'a self,
        path: &'a Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            use tokio::fs;

            if path.is_dir() {
                let mut entries_to_process = Vec::new();
                let mut entries = fs::read_dir(path).await?;

                // Collect all entries first
                while let Some(entry) = entries.next_entry().await? {
                    entries_to_process.push(entry.path());
                }

                // Process directories recursively
                for entry_path in &entries_to_process {
                    if entry_path.is_dir() {
                        self.cleanup_empty_dirs(entry_path).await?;
                    }
                }

                // Re-check if directory is now empty
                let mut entries = fs::read_dir(path).await?;
                if entries.next_entry().await?.is_none() {
                    if std::env::var("DEBUG").is_ok() {
                        eprintln!(
                            "Autotools cleanup_empty_dirs: Removing empty directory: {}",
                            path.display()
                        );
                    }
                    fs::remove_dir(path).await?;
                }
            }

            Ok(())
        })
    }
}

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

        // Add prefix
        if !user_args.iter().any(|arg| arg.starts_with("--prefix=")) {
            args.push(format!("--prefix={}", ctx.prefix.display()));
        }

        // Add DESTDIR support
        args.push(format!("DESTDIR={}", ctx.env.staging_dir().display()));

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
        if let Some(ldflags) = ctx.get_all_env_vars().get("LDFLAGS") {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildContext as RootBuildContext, BuildEnvironment};
    use sps2_types::Version;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_autotools_detection() {
        let temp = tempdir().unwrap();
        let system = AutotoolsBuildSystem::new();

        // Test with configure script
        fs::write(temp.path().join("configure"), "#!/bin/sh\n")
            .await
            .unwrap();
        assert!(system.detect(temp.path()).await.unwrap());

        // Test with configure.ac
        fs::remove_file(temp.path().join("configure"))
            .await
            .unwrap();
        fs::write(temp.path().join("configure.ac"), "AC_INIT\n")
            .await
            .unwrap();
        assert!(system.detect(temp.path()).await.unwrap());

        // Test with Makefile.am
        fs::remove_file(temp.path().join("configure.ac"))
            .await
            .unwrap();
        fs::write(temp.path().join("Makefile.am"), "bin_PROGRAMS = test\n")
            .await
            .unwrap();
        assert!(system.detect(temp.path()).await.unwrap());
    }

    #[tokio::test]
    async fn test_needs_autoreconf() {
        let temp = tempdir().unwrap();
        let system = AutotoolsBuildSystem::new();

        // No configure script - needs autoreconf
        fs::write(temp.path().join("configure.ac"), "AC_INIT\n")
            .await
            .unwrap();
        assert!(system.needs_autoreconf(temp.path()).await.unwrap());

        // Configure exists and is newer - no autoreconf needed
        fs::write(temp.path().join("configure"), "#!/bin/sh\n")
            .await
            .unwrap();
        // Sleep to ensure configure is newer
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert!(!system.needs_autoreconf(temp.path()).await.unwrap());
    }

    #[test]
    fn test_configure_args() {
        let temp = tempdir().unwrap();
        let root_ctx = RootBuildContext::new(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );
        let env = BuildEnvironment::new(root_ctx, temp.path()).unwrap();
        let ctx = BuildSystemContext::new(env, temp.path().to_path_buf());
        let system = AutotoolsBuildSystem::new();

        let args = system.get_configure_args(&ctx, &["--enable-shared".to_string()]);
        assert!(args
            .iter()
            .any(|arg| arg.starts_with("--prefix=/opt/pm/live")));
        assert!(args.iter().any(|arg| arg.starts_with("DESTDIR=")));
        assert!(args.iter().any(|arg| arg == "--enable-shared"));
    }
}

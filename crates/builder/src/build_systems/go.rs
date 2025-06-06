//! Go build system implementation

use super::{BuildSystem, BuildSystemConfig, BuildSystemContext, TestFailure, TestResults};
use async_trait::async_trait;
use sps2_errors::{BuildError, Error};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

/// Go build system
pub struct GoBuildSystem {
    config: BuildSystemConfig,
}

impl GoBuildSystem {
    /// Create a new Go build system instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: BuildSystemConfig {
                supports_out_of_source: false,
                supports_parallel_builds: true,
                supports_incremental_builds: true,
                default_configure_args: vec![],
                default_build_args: vec![],
                env_prefix: Some("GO".to_string()),
                watch_patterns: vec![
                    "go.mod".to_string(),
                    "go.sum".to_string(),
                    "**/*.go".to_string(),
                    "vendor/**".to_string(),
                ],
            },
        }
    }

    /// Setup Go module vendoring for offline builds
    async fn setup_vendoring(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        // Check if vendor directory exists
        let vendor_dir = ctx.source_dir.join("vendor");

        if !vendor_dir.exists() && ctx.network_allowed {
            // Download dependencies and create vendor directory
            let result = ctx
                .execute("go", &["mod", "vendor"], Some(&ctx.source_dir))
                .await?;

            if !result.success {
                return Err(BuildError::ConfigureFailed {
                    message: format!("go mod vendor failed: {}", result.stderr),
                }
                .into());
            }
        }

        Ok(())
    }

    /// Check if this is a Go module project
    async fn is_go_module(&self, source_dir: &Path) -> bool {
        source_dir.join("go.mod").exists()
    }

    /// Get the module name from go.mod
    #[allow(dead_code)]
    async fn get_module_name(&self, ctx: &BuildSystemContext) -> Result<String, Error> {
        let go_mod = ctx.source_dir.join("go.mod");
        if !go_mod.exists() {
            return Ok("main".to_string());
        }

        let content = fs::read_to_string(&go_mod).await?;
        for line in content.lines() {
            if let Some(module_line) = line.strip_prefix("module ") {
                return Ok(module_line.trim().to_string());
            }
        }

        Ok("main".to_string())
    }

    /// Get build arguments for go build
    fn get_build_args(&self, ctx: &BuildSystemContext, user_args: &[String]) -> Vec<String> {
        let mut args = vec!["build".to_string()];

        // Add -mod=vendor if vendor directory exists
        let vendor_dir = ctx.source_dir.join("vendor");
        if vendor_dir.exists() && !user_args.iter().any(|arg| arg.starts_with("-mod=")) {
            args.push("-mod=vendor".to_string());
        }

        // Add build flags for release builds
        if !user_args.contains(&"-gcflags".to_string()) {
            args.push("-gcflags=all=-l".to_string()); // Disable inlining for smaller binaries
        }

        if !user_args.contains(&"-ldflags".to_string()) {
            args.push("-ldflags=-s -w".to_string()); // Strip debug info
        }

        // Add parallel compilation
        if ctx.jobs > 1 && !user_args.iter().any(|arg| arg.starts_with("-p=")) {
            args.push(format!("-p={}", ctx.jobs));
        }

        // Add cross-compilation target
        if ctx.cross_compilation.is_some() {
            // GOOS and GOARCH are set via environment variables
            // No need to add them as arguments
        }

        // Add user arguments (before -o to allow overriding)
        args.extend(user_args.iter().cloned());

        // Don't add -o if user already specified it
        if !args.iter().any(|arg| arg == "-o") {
            // Determine output binary name from build context
            let binary_name = ctx.env.package_name();
            
            // Add output file path
            args.push("-o".to_string());
            let staging_dir = ctx.env.staging_dir();
            let output_path = staging_dir.join("bin").join(binary_name);
            args.push(output_path.display().to_string());
        }

        // Add build target (current directory by default)
        if !user_args
            .iter()
            .any(|arg| arg.ends_with(".go") || arg.contains('/') || arg == "." || arg == "./...")
        {
            args.push(".".to_string()); // Build current package
        }

        args
    }

    /// Parse go test output
    fn parse_test_output(&self, output: &str) -> (usize, usize, usize, Vec<TestFailure>) {
        let mut total = 0;
        let mut passed = 0;
        let mut failed = 0;
        let mut failures = vec![];
        let mut current_package = String::new();

        for line in output.lines() {
            if line.starts_with("=== RUN") {
                total += 1;
            } else if line.starts_with("--- PASS:") {
                passed += 1;
            } else if line.starts_with("--- FAIL:") {
                failed += 1;
                if let Some(test_name) = line
                    .strip_prefix("--- FAIL: ")
                    .and_then(|s| s.split_whitespace().next())
                {
                    failures.push(TestFailure {
                        name: format!("{current_package}/{test_name}"),
                        message: line.to_string(),
                        details: None,
                    });
                }
            } else if line.starts_with("--- SKIP:") {
                // Skipped tests don't count toward total in our model
            } else if line.starts_with("FAIL\t") || line.starts_with("ok  \t") {
                // Package result line
                if let Some(pkg) = line.split('\t').nth(1) {
                    current_package = pkg.to_string();
                }
            }
        }

        // If we didn't find individual test results, check for summary
        if total == 0 && output.contains("PASS") {
            // Assume at least one test passed
            total = 1;
            passed = 1;
        } else if total == 0 && output.contains("FAIL") {
            // Assume at least one test failed
            total = 1;
            failed = 1;
        }

        (total, passed, failed, failures)
    }
}

impl Default for GoBuildSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BuildSystem for GoBuildSystem {
    async fn detect(&self, source_dir: &Path) -> Result<bool, Error> {
        // Check for go.mod (modern Go modules)
        if source_dir.join("go.mod").exists() {
            return Ok(true);
        }

        // Check for any .go files
        let mut entries = fs::read_dir(source_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("go") {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn get_config_options(&self) -> BuildSystemConfig {
        self.config.clone()
    }

    async fn configure(&self, ctx: &BuildSystemContext, _args: &[String]) -> Result<(), Error> {
        // Go doesn't have a configure step, but we can prepare the environment

        // Check Go version
        let result = ctx.execute("go", &["version"], None).await?;
        if !result.success {
            return Err(BuildError::ConfigureFailed {
                message: "go not found in PATH".to_string(),
            }
            .into());
        }

        // Setup vendoring if needed
        if self.is_go_module(&ctx.source_dir).await {
            self.setup_vendoring(ctx).await?;
        }

        // Initialize go.mod if it doesn't exist but we have .go files
        let go_mod = ctx.source_dir.join("go.mod");
        if !go_mod.exists() {
            let module_name = ctx
                .source_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("main");

            let result = ctx
                .execute("go", &["mod", "init", module_name], Some(&ctx.source_dir))
                .await?;

            if !result.success {
                // Non-fatal: old-style GOPATH project
                eprintln!("Warning: go mod init failed, continuing with GOPATH mode");
            }
        }

        Ok(())
    }

    async fn build(&self, ctx: &BuildSystemContext, args: &[String]) -> Result<(), Error> {
        // Create output directory
        let staging_dir = ctx.env.staging_dir();
        let output_dir = staging_dir.join("bin");
        fs::create_dir_all(&output_dir).await?;

        // Get build arguments
        let build_args = self.get_build_args(ctx, args);
        let arg_refs: Vec<&str> = build_args.iter().map(String::as_str).collect();

        // Run go build
        let result = ctx.execute("go", &arg_refs, Some(&ctx.source_dir)).await?;

        if !result.success {
            return Err(BuildError::CompilationFailed {
                message: format!("go build failed: {}", result.stderr),
            }
            .into());
        }

        Ok(())
    }

    async fn test(&self, ctx: &BuildSystemContext) -> Result<TestResults, Error> {
        let start = std::time::Instant::now();

        let mut test_args = vec!["test"];

        // Add vendoring flag if vendor exists
        let vendor_dir = ctx.source_dir.join("vendor");
        if vendor_dir.exists() {
            test_args.push("-mod=vendor");
        }

        // Add verbose flag to get detailed output
        test_args.push("-v");

        // Add parallel test execution
        let jobs_str;
        if ctx.jobs > 1 {
            jobs_str = ctx.jobs.to_string();
            test_args.push("-parallel");
            test_args.push(&jobs_str);
        }

        // Test all packages
        test_args.push("./...");

        // Run go test
        let result = ctx.execute("go", &test_args, Some(&ctx.source_dir)).await?;

        let duration = start.elapsed().as_secs_f64();
        let output = format!("{}\n{}", result.stdout, result.stderr);

        // Parse test results
        let (total, passed, failed, failures) = self.parse_test_output(&output);

        Ok(TestResults {
            total,
            passed,
            failed,
            skipped: 0, // Go doesn't report skipped in the same way
            duration,
            output,
            failures,
        })
    }

    async fn install(&self, ctx: &BuildSystemContext) -> Result<(), Error> {
        // Go build already outputs to the staging directory
        // Just verify the binaries exist
        let staging_dir = ctx.env.staging_dir();
        let bin_dir = staging_dir.join("bin");

        if !bin_dir.exists() {
            return Err(BuildError::InstallFailed {
                message: "No binaries found in staging/bin".to_string(),
            }
            .into());
        }

        // Make sure binaries are executable
        let mut entries = fs::read_dir(&bin_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let metadata = fs::metadata(&path).await?;
                    let mut perms = metadata.permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&path, perms).await?;
                }
            }
        }

        Ok(())
    }

    fn get_env_vars(&self, ctx: &BuildSystemContext) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        // Set GOPATH to deps directory
        vars.insert(
            "GOPATH".to_string(),
            ctx.env.deps_prefix().join("go").display().to_string(),
        );

        // Disable CGO by default for static binaries
        let has_cgo_enabled = if let Ok(extra_env) = ctx.extra_env.read() {
            extra_env.contains_key("CGO_ENABLED")
        } else {
            false
        };
        if !has_cgo_enabled {
            vars.insert("CGO_ENABLED".to_string(), "0".to_string());
        }

        // Set module proxy for offline builds
        if !ctx.network_allowed {
            vars.insert("GOPROXY".to_string(), "off".to_string());
            vars.insert("GONOPROXY".to_string(), "none".to_string());
            vars.insert("GONOSUMDB".to_string(), "*".to_string());
            vars.insert("GOPRIVATE".to_string(), "*".to_string());
        }

        // Cross-compilation support
        if let Some(cross) = &ctx.cross_compilation {
            let goos = match cross.host_platform.os.as_str() {
                "darwin" => "darwin",
                "linux" => "linux",
                "windows" => "windows",
                "freebsd" => "freebsd",
                _ => &cross.host_platform.os,
            };
            vars.insert("GOOS".to_string(), goos.to_string());

            let goarch = match cross.host_platform.arch.as_str() {
                "x86_64" | "amd64" => "amd64",
                "aarch64" | "arm64" => "arm64",
                "i386" | "i686" => "386",
                "armv7" => "arm",
                _ => &cross.host_platform.arch,
            };
            vars.insert("GOARCH".to_string(), goarch.to_string());

            // Additional ARM-specific variables
            if goarch == "arm" {
                vars.insert("GOARM".to_string(), "7".to_string());
            }

            // Re-enable CGO for cross-compilation if needed
            let cgo_enabled = if let Ok(extra_env) = ctx.extra_env.read() {
                extra_env.get("CGO_ENABLED") == Some(&"1".to_string())
            } else {
                false
            };
            if cgo_enabled {
                vars.insert("CC".to_string(), cross.toolchain.cc.clone());
                vars.insert("CXX".to_string(), cross.toolchain.cxx.clone());
                vars.insert("AR".to_string(), cross.toolchain.ar.clone());
            }
        }

        // Set GOCACHE for build caching
        if let Some(cache_config) = &ctx.cache_config {
            vars.insert(
                "GOCACHE".to_string(),
                cache_config
                    .cache_dir
                    .join("go-build")
                    .display()
                    .to_string(),
            );
        }

        vars
    }

    fn name(&self) -> &'static str {
        "go"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildContext as RootBuildContext, BuildEnvironment};
    use sps2_types::Version;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_go_detection() {
        let temp = tempdir().unwrap();
        let system = GoBuildSystem::new();

        // No Go files - should not detect
        assert!(!system.detect(temp.path()).await.unwrap());

        // Create go.mod
        fs::write(temp.path().join("go.mod"), "module test\n\ngo 1.19\n")
            .await
            .unwrap();
        assert!(system.detect(temp.path()).await.unwrap());

        // Remove go.mod, add .go file
        fs::remove_file(temp.path().join("go.mod")).await.unwrap();
        fs::write(temp.path().join("main.go"), "package main\n")
            .await
            .unwrap();
        assert!(system.detect(temp.path()).await.unwrap());
    }

    #[test]
    fn test_build_args() {
        let temp = tempdir().unwrap();
        let root_ctx = RootBuildContext::new(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );
        let env = BuildEnvironment::new(root_ctx, temp.path()).unwrap();
        let ctx = BuildSystemContext::new(env, temp.path().to_path_buf());
        let system = GoBuildSystem::new();

        let args = system.get_build_args(&ctx, &["-tags=release".to_string()]);

        assert!(args.contains(&"build".to_string()));
        assert!(args.contains(&"-ldflags=-s -w".to_string()));
        assert!(args.contains(&"-tags=release".to_string()));
        assert!(args.contains(&"./...".to_string()));
    }

    #[test]
    fn test_parse_test_output() {
        let system = GoBuildSystem::new();
        let output = r"
=== RUN   TestExample
--- PASS: TestExample (0.00s)
=== RUN   TestFailure
--- FAIL: TestFailure (0.01s)
    main_test.go:10: expected true, got false
PASS
ok      example.com/test    0.123s
";

        let (total, passed, failed, failures) = system.parse_test_output(output);
        assert_eq!(total, 2);
        assert_eq!(passed, 1);
        assert_eq!(failed, 1);
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn test_env_vars() {
        let temp = tempdir().unwrap();
        let root_ctx = RootBuildContext::new(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );
        let env = BuildEnvironment::new(root_ctx, temp.path()).unwrap();
        let ctx = BuildSystemContext::new(env, temp.path().to_path_buf());
        let system = GoBuildSystem::new();

        let vars = system.get_env_vars(&ctx);
        assert!(vars.contains_key("GOPATH"));
        assert_eq!(vars.get("CGO_ENABLED"), Some(&"0".to_string()));
        assert_eq!(vars.get("GOPROXY"), Some(&"off".to_string()));
    }
}

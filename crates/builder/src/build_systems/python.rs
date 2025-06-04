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

    /// Create virtual environment for isolated builds
    async fn create_venv(&self, ctx: &BuildSystemContext) -> Result<PathBuf, Error> {
        let venv_path = ctx.build_dir.join("venv");

        // Create virtual environment
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
    fn get_pip_args(&self, ctx: &BuildSystemContext, user_args: &[String]) -> Vec<String> {
        let mut args = vec!["install".to_string()];

        // Add offline mode if network is disabled
        if !ctx.network_allowed {
            args.push("--no-index".to_string());
            args.push("--find-links".to_string());
            args.push(ctx.source_dir.join("vendor").display().to_string());
        }

        // Add prefix for installation
        args.push("--prefix".to_string());
        args.push(ctx.env.staging_dir().display().to_string());

        // Add no-deps to avoid installing dependencies (they should be handled by sps2)
        if !user_args.contains(&"--no-deps".to_string()) {
            args.push("--no-deps".to_string());
        }

        // Add user arguments
        args.extend(user_args.iter().cloned());

        args
    }

    /// Build wheel using PEP 517
    async fn build_wheel_pep517(&self, ctx: &BuildSystemContext) -> Result<PathBuf, Error> {
        let wheel_dir = ctx.build_dir.join("dist");
        fs::create_dir_all(&wheel_dir).await?;

        // Install build dependencies
        let result = ctx
            .execute(
                "pip",
                &[
                    "install",
                    "--upgrade",
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

        // Build wheel using python-build
        let wheel_dir_str = wheel_dir.display().to_string();
        let mut build_args = vec!["-m", "build", "--wheel", "--outdir", &wheel_dir_str];

        if !ctx.network_allowed {
            build_args.push("--no-isolation");
        }

        let result = ctx
            .execute("python3", &build_args, Some(&ctx.source_dir))
            .await?;

        if !result.success {
            return Err(BuildError::CompilationFailed {
                message: format!("Failed to build wheel: {}", result.stderr),
            }
            .into());
        }

        // Find the built wheel
        let mut entries = fs::read_dir(&wheel_dir).await?;
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
    fn parse_pytest_output(&self, output: &str) -> (usize, usize, usize, Vec<TestFailure>) {
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

        // Create virtual environment for isolation
        if ctx.build_dir != ctx.source_dir {
            self.create_venv(ctx).await?;
        }

        // Store detected backend in environment for later use
        if let Ok(mut extra_env) = ctx.extra_env.write() {
            extra_env.insert("PYTHON_BUILD_BACKEND".to_string(), format!("{:?}", backend));
        }

        Ok(())
    }

    async fn build(&self, ctx: &BuildSystemContext, _args: &[String]) -> Result<(), Error> {
        // Build wheel using PEP 517
        let wheel_path = self.build_wheel_pep517(ctx).await?;

        // Store wheel path for install phase
        if let Ok(mut extra_env) = ctx.extra_env.write() {
            extra_env.insert(
                "PYTHON_WHEEL_PATH".to_string(),
                wheel_path.display().to_string(),
            );
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

        if !pytest_check.map(|r| r.success).unwrap_or(false) {
            // Fall back to unittest
            test_cmd = "python3";
            test_args = vec![
                "-m".to_string(),
                "unittest".to_string(),
                "discover".to_string(),
            ];
        } else {
            // Use pytest with verbose output
            let mut args = vec!["-v".to_string(), "--tb=short".to_string()];

            // Add parallel execution if supported
            if ctx.jobs > 1 {
                args.push("-n".to_string());
                jobs_str = ctx.jobs.to_string();
                args.push(jobs_str.clone());
            }
            test_args = args;
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
            self.parse_pytest_output(&output)
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
        // Get wheel path from build phase
        let wheel_path = if let Ok(extra_env) = ctx.extra_env.read() {
            extra_env.get("PYTHON_WHEEL_PATH").cloned().ok_or_else(|| {
                BuildError::InstallFailed {
                    message: "No wheel found from build phase".to_string(),
                }
            })?
        } else {
            return Err(BuildError::InstallFailed {
                message: "Cannot access extra environment".to_string(),
            }
            .into());
        };

        // Install wheel using pip
        let pip_args = self.get_pip_args(ctx, &[wheel_path.clone()]);
        let arg_refs: Vec<&str> = pip_args.iter().map(String::as_str).collect();

        let result = ctx.execute("pip", &arg_refs, Some(&ctx.source_dir)).await?;

        if !result.success {
            return Err(BuildError::InstallFailed {
                message: format!("pip install failed: {}", result.stderr),
            }
            .into());
        }

        // Fix shebangs in installed scripts
        let scripts_dir = ctx.env.staging_dir().join("bin");
        if scripts_dir.exists() {
            self.fix_shebangs(&scripts_dir, ctx).await?;
        }

        Ok(())
    }

    fn get_env_vars(&self, ctx: &BuildSystemContext) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        // Set PYTHONPATH to include staging directory
        let site_packages = ctx.env.staging_dir().join("lib/python*/site-packages");
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
    /// Fix shebangs in Python scripts to use correct Python path
    async fn fix_shebangs(
        &self,
        scripts_dir: &Path,
        _ctx: &BuildSystemContext,
    ) -> Result<(), Error> {
        let mut entries = fs::read_dir(scripts_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                let content = fs::read_to_string(&path).await?;
                if content.starts_with("#!") {
                    // Replace shebang with generic python3
                    let lines: Vec<&str> = content.lines().collect();
                    if !lines.is_empty() && lines[0].contains("python") {
                        let mut new_content = String::from("#!/usr/bin/env python3\n");
                        new_content.push_str(&lines[1..].join("\n"));
                        fs::write(&path, new_content).await?;
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildContext as RootBuildContext, BuildEnvironment};
    use sps2_types::Version;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_python_detection() {
        let temp = tempdir().unwrap();
        let system = PythonBuildSystem::new();

        // No Python files - should not detect
        assert!(!system.detect(temp.path()).await.unwrap());

        // Create pyproject.toml
        fs::write(
            temp.path().join("pyproject.toml"),
            "[build-system]\nrequires = [\"setuptools\"]\n",
        )
        .await
        .unwrap();
        assert!(system.detect(temp.path()).await.unwrap());

        // Create setup.py
        fs::remove_file(temp.path().join("pyproject.toml"))
            .await
            .unwrap();
        fs::write(
            temp.path().join("setup.py"),
            "from setuptools import setup\nsetup()\n",
        )
        .await
        .unwrap();
        assert!(system.detect(temp.path()).await.unwrap());
    }

    #[tokio::test]
    async fn test_build_backend_detection() {
        let temp = tempdir().unwrap();
        let system = PythonBuildSystem::new();

        // Test setuptools detection
        fs::write(
            temp.path().join("pyproject.toml"),
            "[build-system]\nrequires = [\"setuptools>=40\"]\n",
        )
        .await
        .unwrap();

        match system.detect_build_backend(temp.path()).await {
            Ok(BuildBackend::Setuptools) => {}
            _ => panic!("Expected Setuptools backend"),
        }

        // Test poetry detection
        fs::write(
            temp.path().join("pyproject.toml"),
            "[build-system]\nrequires = [\"poetry-core\"]\n",
        )
        .await
        .unwrap();

        match system.detect_build_backend(temp.path()).await {
            Ok(BuildBackend::Poetry) => {}
            _ => panic!("Expected Poetry backend"),
        }
    }

    #[test]
    fn test_parse_pytest_output() {
        let system = PythonBuildSystem::new();
        let output = r"
============================= test session starts ==============================
collected 10 items

test_example.py::test_one PASSED                                         [ 10%]
test_example.py::test_two PASSED                                         [ 20%]
test_example.py::test_three FAILED                                       [ 30%]
test_example.py::test_four SKIPPED                                       [ 40%]

=========================== short test summary info ============================
FAILED test_example.py::test_three - AssertionError: assert False
=================== 2 passed, 1 failed, 1 skipped in 0.12s ====================
";

        let (total, passed, failed, failures) = system.parse_pytest_output(output);
        assert_eq!(total, 4);
        assert_eq!(passed, 2);
        assert_eq!(failed, 1);
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn test_pip_args() {
        let temp = tempdir().unwrap();
        let root_ctx = RootBuildContext::new(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );
        let env = BuildEnvironment::new(root_ctx, temp.path()).unwrap();
        let ctx = BuildSystemContext::new(env, temp.path().to_path_buf());
        let system = PythonBuildSystem::new();

        let args = system.get_pip_args(&ctx, &["--verbose".to_string()]);

        assert!(args.contains(&"install".to_string()));
        assert!(args.contains(&"--no-deps".to_string()));
        assert!(args.contains(&"--no-index".to_string()));
        assert!(args.contains(&"--verbose".to_string()));
    }
}

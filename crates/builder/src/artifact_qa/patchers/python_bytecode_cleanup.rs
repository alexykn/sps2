//! Patcher that removes Python bytecode and build artifacts from staging directory
//!
//! This patcher cleans up dynamic files generated during Python package installation
//! that should not be included in the final .sp packages. These files are automatically
//! regenerated when Python packages are used at runtime.

use crate::artifact_qa::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::{AppEvent, QaEvent};
use std::path::Path;
use tokio::fs;

#[derive(Default)]
pub struct PythonBytecodeCleanupPatcher;

impl PythonBytecodeCleanupPatcher {
    /// Remove all __pycache__ directories and their contents
    async fn remove_pycache_dirs(
        &self,
        staging_dir: &Path,
    ) -> Result<Vec<std::path::PathBuf>, Error> {
        let mut removed_dirs = Vec::new();

        for entry in ignore::WalkBuilder::new(staging_dir)
            .hidden(false)
            .parents(false)
            .build()
        {
            let path = match entry {
                Ok(e) => e.into_path(),
                Err(_) => continue,
            };

            if path.is_dir() && path.file_name().and_then(|n| n.to_str()) == Some("__pycache__") {
                if let Ok(()) = fs::remove_dir_all(&path).await {
                    removed_dirs.push(path);
                }
            }
        }

        Ok(removed_dirs)
    }

    /// Remove individual bytecode files (.pyc, .pyo, etc.)
    async fn remove_bytecode_files(
        &self,
        staging_dir: &Path,
    ) -> Result<Vec<std::path::PathBuf>, Error> {
        let mut removed_files = Vec::new();

        for entry in ignore::WalkBuilder::new(staging_dir)
            .hidden(false)
            .parents(false)
            .build()
        {
            let path = match entry {
                Ok(e) => e.into_path(),
                Err(_) => continue,
            };

            if !path.is_file() {
                continue;
            }

            // Check for bytecode file extensions
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if matches!(ext, "pyc" | "pyo") {
                    if let Ok(()) = fs::remove_file(&path).await {
                        removed_files.push(path);
                    }
                    continue;
                }
            }

            // Check for complex bytecode patterns (.cpython-*.pyc, etc.)
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                let has_pyc_extension = path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pyc"));

                if has_pyc_extension
                    && (filename.contains(".cpython-")
                        || filename.contains(".pypy")
                        || filename.contains(".opt-"))
                {
                    if let Ok(()) = fs::remove_file(&path).await {
                        removed_files.push(path);
                    }
                }
            }
        }

        Ok(removed_files)
    }

    /// Remove build artifacts and development files
    async fn remove_build_artifacts(
        &self,
        staging_dir: &Path,
    ) -> Result<Vec<std::path::PathBuf>, Error> {
        let mut removed_items = Vec::new();

        for entry in ignore::WalkBuilder::new(staging_dir)
            .hidden(false)
            .parents(false)
            .build()
        {
            let path = match entry {
                Ok(e) => e.into_path(),
                Err(_) => continue,
            };

            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            // Remove common build artifacts and development directories
            let should_remove = matches!(
                name,
                "build"
                    | "dist"
                    | ".eggs"
                    | ".tox"
                    | ".pytest_cache"
                    | ".mypy_cache"
                    | ".ruff_cache"
                    | "htmlcov"
                    | "tests"
                    | "test"
                    | "docs"
                    | "examples"
                    | ".DS_Store"
                    | "Thumbs.db"
                    | ".vscode"
                    | ".idea"
            ) || name.ends_with(".egg-info")
                || name.starts_with("pip-build-env-")
                || name.starts_with("pip-req-build-")
                || name.contains("pip-")
                || name.starts_with("test_")
                || name.ends_with("_test.py")
                || name.ends_with("_tests.py")
                || name == "conftest.py";

            if should_remove {
                let remove_result = if path.is_dir() {
                    fs::remove_dir_all(&path).await
                } else {
                    fs::remove_file(&path).await
                };

                if remove_result.is_ok() {
                    removed_items.push(path);
                }
            }
        }

        Ok(removed_items)
    }

    /// Remove pip cache and metadata files
    async fn remove_pip_artifacts(
        &self,
        staging_dir: &Path,
    ) -> Result<Vec<std::path::PathBuf>, Error> {
        let mut removed_items = Vec::new();

        for entry in ignore::WalkBuilder::new(staging_dir)
            .hidden(false)
            .parents(false)
            .build()
        {
            let path = match entry {
                Ok(e) => e.into_path(),
                Err(_) => continue,
            };

            if !path.is_file() {
                continue;
            }

            let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            // Remove pip installation metadata that contains hardcoded paths
            if matches!(filename, "INSTALLER" | "REQUESTED" | "direct_url.json") {
                if let Ok(()) = fs::remove_file(&path).await {
                    removed_items.push(path);
                }
            }
        }

        Ok(removed_items)
    }
}

impl crate::artifact_qa::traits::Action for PythonBytecodeCleanupPatcher {
    const NAME: &'static str = "Python Bytecode Cleanup";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&crate::artifact_qa::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
        // Only run for Python packages
        if !env.is_python_package() {
            return Ok(Report::ok());
        }

        let self_instance = Self;
        let staging_dir = env.staging_dir();
        let mut all_removed = Vec::new();

        // Remove __pycache__ directories
        let removed_pycache = self_instance.remove_pycache_dirs(staging_dir).await?;
        all_removed.extend(removed_pycache);

        // Remove bytecode files
        let removed_bytecode = self_instance.remove_bytecode_files(staging_dir).await?;
        all_removed.extend(removed_bytecode);

        // Remove build artifacts
        let removed_artifacts = self_instance.remove_build_artifacts(staging_dir).await?;
        all_removed.extend(removed_artifacts);

        // Remove pip metadata
        let removed_pip = self_instance.remove_pip_artifacts(staging_dir).await?;
        all_removed.extend(removed_pip);

        // Report results
        if !all_removed.is_empty() {
            crate::utils::events::send_event(
                ctx,
                AppEvent::Qa(QaEvent::CheckCompleted {
                    check_type: "patcher".to_string(),
                    check_name: "python_bytecode_cleanup".to_string(),
                    findings_count: all_removed.len(),
                    severity_counts: std::collections::HashMap::new(),
                }),
            );
        }

        Ok(Report {
            changed_files: all_removed,
            ..Default::default()
        })
    }
}

impl Patcher for PythonBytecodeCleanupPatcher {}

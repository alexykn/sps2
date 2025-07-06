//! Validator that checks if the staging directory contains any files.
//!
//! This is a fundamental check that runs for all build system profiles.
//! An empty staging directory indicates that the build succeeded but no files
//! were installed, which usually means the install step failed or was skipped.

use crate::artifact_qa::{diagnostics::DiagnosticCollector, reports::Report, traits::Validator};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use std::path::Path;

pub struct StagingScanner;

impl crate::artifact_qa::traits::Action for StagingScanner {
    const NAME: &'static str = "Staging directory scanner";

    async fn run(
        _ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&DiagnosticCollector>,
    ) -> Result<Report, Error> {
        let staging_dir = env.staging_dir();

        // Check if staging directory exists and has any content
        if !staging_dir.exists() {
            let mut report = Report::default();
            report.errors.push(format!(
                "Staging directory does not exist: {}",
                staging_dir.display()
            ));
            return Ok(report);
        }

        // Check if staging directory is empty
        if is_directory_empty(staging_dir)? {
            let mut report = Report::default();
            report.errors.push(format!(
                "Staging directory is empty: {}. This usually indicates that the build's install step failed or was not run. Check the build recipe for proper 'make install' or equivalent installation commands.",
                staging_dir.display()
            ));
            return Ok(report);
        }

        // Staging directory has content - success
        Ok(Report::ok())
    }
}

impl Validator for StagingScanner {}

/// Check if a directory is empty (has no files or subdirectories)
fn is_directory_empty(dir: &Path) -> Result<bool, Error> {
    let mut entries =
        std::fs::read_dir(dir).map_err(|e| sps2_errors::BuildError::ValidationFailed {
            message: format!("Failed to read staging directory {}: {}", dir.display(), e),
        })?;

    // If we can get even one entry, the directory is not empty
    Ok(entries.next().is_none())
}

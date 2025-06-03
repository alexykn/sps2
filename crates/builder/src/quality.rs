//! Quality checks and relocatability validation for built packages

use crate::events::send_event;
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use std::path::Path;
use tokio::fs;

/// Run quality checks on the built package
pub async fn run_quality_checks(
    context: &BuildContext,
    environment: &BuildEnvironment,
) -> Result<(), Error> {
    // Scan for hardcoded paths (relocatability check)
    scan_for_hardcoded_paths(context, environment).await
}

/// Scan staging directory for hardcoded build paths (relocatability check)
pub async fn scan_for_hardcoded_paths(
    context: &BuildContext,
    environment: &BuildEnvironment,
) -> Result<(), Error> {
    let staging_dir = environment.staging_dir();
    let build_prefix = environment.build_prefix();
    let build_prefix_str = build_prefix.display().to_string();

    send_event(
        context,
        Event::OperationStarted {
            operation: "Scanning for hardcoded paths".to_string(),
        },
    );

    // Skip scanning if staging directory doesn't exist or is empty
    if !staging_dir.exists() {
        send_event(
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
        send_event(
            context,
            Event::OperationCompleted {
                operation: "Relocatability scan skipped (empty staging directory)".to_string(),
                success: true,
            },
        );
        return Ok(());
    }

    let violations = scan_directory_for_hardcoded_paths(staging_dir, &build_prefix_str).await?;

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

    send_event(
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
                let mut sub_violations =
                    scan_directory_for_hardcoded_paths(&path, build_prefix).await?;
                violations.append(&mut sub_violations);
            } else if path.is_file() {
                // Check file for hardcoded paths
                if let Some(violation) = scan_file_for_hardcoded_paths(&path, build_prefix).await? {
                    violations.push(violation);
                }
            }
        }

        Ok(violations)
    })
}

/// Scan individual file for hardcoded paths
async fn scan_file_for_hardcoded_paths(
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

//! Quality checks and relocatability validation for built packages

use crate::events::send_event;
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Run quality checks on the built package
pub async fn run_quality_checks(
    context: &BuildContext,
    environment: &BuildEnvironment,
) -> Result<(), Error> {
    // Remove .la files (obsolete on macOS and cause relocatability issues)
    remove_la_files(context, environment).await?;

    // Fix RPATH issues on macOS binaries if needed
    if cfg!(target_os = "macos") {
        fix_macos_rpaths(context, environment).await?;
    }

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

/// Remove .la files from the staging directory
///
/// .la files are libtool archives that are obsolete on macOS and contain
/// hardcoded build paths that break relocatability. Major package managers
/// like Homebrew actively remove them.
async fn remove_la_files(
    context: &BuildContext,
    environment: &BuildEnvironment,
) -> Result<(), Error> {
    let staging_dir = environment.staging_dir();

    // Skip if staging directory doesn't exist
    if !staging_dir.exists() {
        return Ok(());
    }

    // Find all .la files
    let mut la_files = Vec::new();
    find_la_files_recursive(staging_dir, &mut la_files).await?;

    if la_files.is_empty() {
        return Ok(());
    }

    send_event(
        context,
        Event::OperationStarted {
            operation: format!("Removing {} obsolete .la files", la_files.len()),
        },
    );

    // Remove all .la files
    for file in &la_files {
        fs::remove_file(file).await?;
    }

    send_event(
        context,
        Event::OperationCompleted {
            operation: format!("Removed {} .la files", la_files.len()),
            success: true,
        },
    );

    Ok(())
}

/// Recursively find all .la files in a directory
fn find_la_files_recursive<'a>(
    dir: &'a Path,
    files: &'a mut Vec<PathBuf>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + 'a>> {
    Box::pin(async move {
        if !dir.exists() {
            return Ok(());
        }

        let Ok(mut entries) = fs::read_dir(dir).await else {
            return Ok(());
        };

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_dir() {
                // Recursively search subdirectories
                find_la_files_recursive(&path, files).await?;
            } else if path.extension().and_then(|e| e.to_str()) == Some("la") {
                // Found a .la file
                files.push(path);
            }
        }

        Ok(())
    })
}

/// Fix RPATH issues on macOS binaries
///
/// This ensures all executables and libraries have proper RPATH entries
/// to find their dependencies at runtime.
async fn fix_macos_rpaths(
    context: &BuildContext,
    environment: &BuildEnvironment,
) -> Result<(), Error> {
    use tokio::process::Command;

    let staging_dir = environment.staging_dir();
    let live_prefix = environment.build_prefix();
    let lib_path = format!("{}/lib", live_prefix.display());

    // Skip if staging directory doesn't exist
    if !staging_dir.exists() {
        return Ok(());
    }

    send_event(
        context,
        Event::OperationStarted {
            operation: "Checking and fixing RPATH entries".to_string(),
        },
    );

    // Find all executables and dynamic libraries
    let mut binaries = Vec::new();
    find_binaries_recursive(staging_dir, &mut binaries).await?;

    let mut fixed_count = 0;

    for binary_path in &binaries {
        // Check if binary needs RPATH fix using otool
        let output = Command::new("otool")
            .args(["-l", &binary_path.display().to_string()])
            .output()
            .await?;

        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);

            // Check if it has LC_RPATH pointing to our lib directory
            let has_correct_rpath =
                output_str.contains("LC_RPATH") && output_str.contains(&lib_path);

            // Check if it has @rpath references (needs RPATH)
            let needs_rpath = output_str.contains("@rpath/");

            if needs_rpath && !has_correct_rpath {
                // Add RPATH using install_name_tool
                let result = Command::new("install_name_tool")
                    .args(["-add_rpath", &lib_path, &binary_path.display().to_string()])
                    .output()
                    .await?;

                if result.status.success() {
                    fixed_count += 1;
                } else {
                    // Log warning but don't fail the build
                    send_event(
                        context,
                        Event::Warning {
                            message: format!(
                                "Failed to add RPATH to {}: {}",
                                binary_path.display(),
                                String::from_utf8_lossy(&result.stderr)
                            ),
                            context: Some("RPATH fix".to_string()),
                        },
                    );
                }
            }
        }
    }

    if fixed_count > 0 {
        send_event(
            context,
            Event::OperationCompleted {
                operation: format!("Fixed RPATH entries for {} binaries", fixed_count),
                success: true,
            },
        );
    } else {
        send_event(
            context,
            Event::OperationCompleted {
                operation: "All RPATH entries are correct".to_string(),
                success: true,
            },
        );
    }

    Ok(())
}

/// Recursively find all binary files (executables and libraries)
fn find_binaries_recursive<'a>(
    dir: &'a Path,
    files: &'a mut Vec<PathBuf>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + 'a>> {
    Box::pin(async move {
        if !dir.exists() {
            return Ok(());
        }

        let Ok(mut entries) = fs::read_dir(dir).await else {
            return Ok(());
        };

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_dir() {
                // Skip certain directories
                if let Some(name) = path.file_name() {
                    let name_str = name.to_string_lossy();
                    if name_str == "share" || name_str == "include" || name_str == "man" {
                        continue;
                    }
                }

                // Recursively search subdirectories
                find_binaries_recursive(&path, files).await?;
            } else if path.is_file() {
                // Check if it's a binary file
                if is_macos_binary(&path).await? {
                    files.push(path);
                }
            }
        }

        Ok(())
    })
}

/// Check if a file is a macOS binary (Mach-O executable or library)
async fn is_macos_binary(path: &Path) -> Result<bool, Error> {
    use tokio::process::Command;

    // Use file command to check if it's a Mach-O binary
    let output = Command::new("file")
        .arg(path.display().to_string())
        .output()
        .await?;

    if output.status.success() {
        let output_str = String::from_utf8_lossy(&output.stdout);
        Ok(output_str.contains("Mach-O")
            && (output_str.contains("executable")
                || output_str.contains("dynamically linked shared library")))
    } else {
        Ok(false)
    }
}

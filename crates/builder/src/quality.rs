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

    // Replace placeholder paths with actual prefix for relocatable packages
    replace_placeholder_paths(context, environment).await?;

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

/// Check binary files (executables and libraries) for hardcoded build paths
async fn check_binary_for_build_paths(
    file_path: &Path,
    build_prefix: &str,
) -> Result<Option<String>, Error> {
    use tokio::process::Command;

    // Use strings command to extract text from binary
    let output = Command::new("strings")
        .arg(file_path.display().to_string())
        .output()
        .await?;

    if output.status.success() {
        let strings_output = String::from_utf8_lossy(&output.stdout);

        // Check if any strings contain the build prefix
        if strings_output.contains(build_prefix) {
            // Also check with otool for dynamic libraries
            if file_path.extension().and_then(|e| e.to_str()) == Some("dylib") {
                // Check RPATHs specifically
                let otool_output = Command::new("otool")
                    .args(["-l", &file_path.display().to_string()])
                    .output()
                    .await?;

                if otool_output.status.success() {
                    let otool_str = String::from_utf8_lossy(&otool_output.stdout);
                    if otool_str.contains(build_prefix) {
                        return Ok(Some(format!(
                            "{} (contains RPATH or load path with '{}')",
                            file_path.display(),
                            build_prefix
                        )));
                    }
                }
            }

            return Ok(Some(format!(
                "{} (contains string '{}')",
                file_path.display(),
                build_prefix
            )));
        }
    }

    Ok(None)
}

/// Scan individual file for hardcoded paths
async fn scan_file_for_hardcoded_paths(
    file_path: &Path,
    build_prefix: &str,
) -> Result<Option<String>, Error> {
    // Skip non-text files and certain file types that are expected to contain paths
    if let Some(extension) = file_path.extension() {
        let ext = extension.to_string_lossy().to_lowercase();
        // Skip truly binary files that might contain false positives
        // But DO check dynamic libraries for hardcoded paths
        if matches!(
            ext.as_str(),
            "a" | "o"
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

        // For dynamic libraries, use special checking
        if matches!(ext.as_str(), "so" | "dylib") {
            return check_binary_for_build_paths(file_path, build_prefix).await;
        }
    }

    // Read file content
    let Ok(content) = fs::read_to_string(file_path).await else {
        // File is not text, skip it (binary files)
        return Ok(None);
    };

    // Check if content contains the build prefix
    // But ignore our placeholder prefix - that's expected and will be replaced
    if content.contains(build_prefix) && !content.contains(crate::BUILD_PLACEHOLDER_PREFIX) {
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
        if fix_single_binary_rpath(context, binary_path, &lib_path).await? {
            fixed_count += 1;
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

/// Fix RPATH entries for a single binary
async fn fix_single_binary_rpath(
    context: &BuildContext,
    binary_path: &Path,
    lib_path: &str,
) -> Result<bool, Error> {
    use tokio::process::Command;

    // Check if binary needs RPATH fix using otool
    let output = Command::new("otool")
        .args(["-l", &binary_path.display().to_string()])
        .output()
        .await?;

    if !output.status.success() {
        return Ok(false);
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let mut has_correct_rpath = false;
    let mut rpaths_to_remove = Vec::new();

    // Parse RPATHs from otool output
    let mut lines = output_str.lines();
    while let Some(line) = lines.next() {
        if line.contains("LC_RPATH") {
            // Skip cmdsize line
            let _ = lines.next();
            // Get the path line
            if let Some(path_line) = lines.next() {
                if path_line.contains("path ") {
                    if let Some(path_start) = path_line.find("path ") {
                        let path_part = &path_line[path_start + 5..];
                        if let Some(space_pos) = path_part.find(" (") {
                            let rpath = &path_part[..space_pos];

                            // Check if this is our correct lib path
                            if rpath == lib_path {
                                has_correct_rpath = true;
                            }
                            // Check if this is a build path that needs removal
                            else if rpath.contains("/opt/pm/build/")
                                || rpath.contains("/opt/homebrew/")
                                || rpath.contains("/usr/local/")
                                || rpath.contains("/tmp/")
                            {
                                rpaths_to_remove.push(rpath.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Remove bad RPATHs
    let mut removed_any = false;
    for bad_rpath in &rpaths_to_remove {
        let result = Command::new("install_name_tool")
            .args([
                "-delete_rpath",
                bad_rpath,
                &binary_path.display().to_string(),
            ])
            .output()
            .await?;

        if result.status.success() {
            removed_any = true;
        }
    }

    // Check if it has @rpath references (needs RPATH)
    let needs_rpath = output_str.contains("@rpath/");

    // Add correct RPATH if needed and not already present
    if needs_rpath && !has_correct_rpath {
        // Add RPATH using install_name_tool
        let result = Command::new("install_name_tool")
            .args(["-add_rpath", lib_path, &binary_path.display().to_string()])
            .output()
            .await?;

        if result.status.success() {
            Ok(true)
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
            Ok(false)
        }
    } else {
        Ok(removed_any)
    }
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

/// Replace placeholder paths with actual installation prefix
async fn replace_placeholder_paths(
    context: &BuildContext,
    environment: &BuildEnvironment,
) -> Result<(), Error> {
    let staging_dir = environment.staging_dir();
    let actual_prefix = "/opt/pm/live";
    let build_prefix = environment.build_prefix().display().to_string();

    // Skip if staging directory doesn't exist
    if !staging_dir.exists() {
        return Ok(());
    }

    send_event(
        context,
        Event::OperationStarted {
            operation: "Replacing placeholder and build paths for relocatable packages".to_string(),
        },
    );

    let mut replaced_count = 0;

    // Recursively find and replace in all files
    replaced_count += replace_in_directory(staging_dir, actual_prefix, &build_prefix).await?;

    if replaced_count > 0 {
        send_event(
            context,
            Event::OperationCompleted {
                operation: format!("Replaced paths in {} files", replaced_count),
                success: true,
            },
        );
    } else {
        send_event(
            context,
            Event::OperationCompleted {
                operation: "No paths found to replace".to_string(),
                success: true,
            },
        );
    }

    Ok(())
}

/// Recursively replace placeholders in directory
fn replace_in_directory<'a>(
    dir: &'a Path,
    actual_prefix: &'a str,
    build_prefix: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<usize, Error>> + 'a>> {
    Box::pin(async move {
        let mut replaced_count = 0;

        if !dir.exists() {
            return Ok(0);
        }

        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_dir() {
                replaced_count += replace_in_directory(&path, actual_prefix, build_prefix).await?;
            } else if path.is_file() {
                // Check if it's a dylib that needs special handling
                if is_dylib(&path) {
                    if update_dylib_paths_quality(&path, actual_prefix, build_prefix).await? {
                        replaced_count += 1;
                    }
                } else if should_replace_in_file(&path).await?
                    && replace_placeholders_in_file(&path, actual_prefix, build_prefix).await?
                {
                    replaced_count += 1;
                }
            }
        }

        Ok(replaced_count)
    })
}

/// Replace placeholders in a single file using native Rust
async fn replace_placeholders_in_file(
    file_path: &Path,
    actual_prefix: &str,
    build_prefix: &str,
) -> Result<bool, Error> {
    // Read the file content
    let content = match fs::read_to_string(file_path).await {
        Ok(content) => content,
        Err(_) => return Ok(false), // Skip binary files
    };

    let mut new_content = content.clone();
    let mut replaced = false;

    // Replace placeholder paths
    if new_content.contains(crate::BUILD_PLACEHOLDER_PREFIX) {
        new_content = new_content.replace(crate::BUILD_PLACEHOLDER_PREFIX, actual_prefix);
        replaced = true;
    }

    // Replace build paths (like /opt/pm/build/package/version/deps)
    if new_content.contains(build_prefix) {
        new_content = new_content.replace(build_prefix, actual_prefix);
        replaced = true;
    }

    // Also replace specific build deps paths
    let build_deps_pattern = format!("{}/deps", build_prefix);
    if new_content.contains(&build_deps_pattern) {
        new_content = new_content.replace(&build_deps_pattern, actual_prefix);
        replaced = true;
    }

    if replaced {
        // Write back to file
        fs::write(file_path, new_content).await?;
    }

    Ok(replaced)
}

/// Check if we should replace placeholders in this file
async fn should_replace_in_file(path: &Path) -> Result<bool, Error> {
    // Get file extension
    if let Some(extension) = path.extension() {
        let ext = extension.to_string_lossy().to_lowercase();

        // Text files we should process
        let text_extensions = [
            "h", "hpp", "hh", "hxx", "h++", // C/C++ headers
            "c", "cpp", "cc", "cxx", "c++", // C/C++ source
            "pc", "cmake", "make", "mk", // Build files
            "py", "rb", "pl", "sh", "bash", // Scripts
            "txt", "conf", "cfg", "ini", // Config files
            "json", "yaml", "yml", "toml", // Data files
            "xml", "plist", // macOS files
        ];

        if text_extensions.contains(&ext.as_str()) {
            return Ok(true);
        }
    }

    // Check files without extensions (like shell scripts)
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.') || !n.contains('.'))
        .unwrap_or(false)
    {
        // Try to detect if it's a text file using the file command
        use tokio::process::Command;

        let output = Command::new("file")
            .arg("--mime-type")
            .arg(path.display().to_string())
            .output()
            .await?;

        if output.status.success() {
            let mime = String::from_utf8_lossy(&output.stdout);
            return Ok(mime.contains("text/"));
        }
    }

    Ok(false)
}

/// Check if a file is a dylib
fn is_dylib(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        ext == "dylib" || path.to_string_lossy().contains(".dylib.")
    } else {
        false
    }
}

/// Update paths in a dylib file for the quality check phase
async fn update_dylib_paths_quality(
    dylib_path: &Path,
    actual_prefix: &str,
    build_prefix: &str,
) -> Result<bool, Error> {
    use tokio::process::Command;

    // First, get the current install name and dependencies
    let output = Command::new("otool")
        .args(["-L", &dylib_path.to_string_lossy()])
        .output()
        .await?;

    if !output.status.success() {
        // If otool fails, skip this file
        return Ok(false);
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = output_str.lines().collect();

    if lines.is_empty() {
        return Ok(false);
    }

    let mut updated = false;

    // Update install name
    if update_dylib_install_name_quality(&lines, dylib_path, actual_prefix, build_prefix).await? {
        updated = true;
    }

    // Update dependency paths
    if update_dylib_dependencies_quality(&lines, dylib_path, actual_prefix, build_prefix).await? {
        updated = true;
    }

    // Update RPATHs
    if update_dylib_rpaths_quality(dylib_path, actual_prefix, build_prefix).await? {
        updated = true;
    }

    Ok(updated)
}

/// Update the install name of a dylib in quality phase
async fn update_dylib_install_name_quality(
    lines: &[&str],
    dylib_path: &Path,
    actual_prefix: &str,
    build_prefix: &str,
) -> Result<bool, Error> {
    use tokio::process::Command;

    // First line after the header is the install name (for dylibs)
    if lines.len() > 1 {
        let first_dep = lines[1].trim();
        if let Some(space_pos) = first_dep.find(" (") {
            let install_name = &first_dep[..space_pos];

            // Check if it contains placeholder, build paths, or live/deps paths
            let live_deps_pattern = format!("{}/deps", actual_prefix);
            let needs_update = install_name.contains(crate::BUILD_PLACEHOLDER_PREFIX)
                || install_name.contains(build_prefix)
                || install_name.contains(&live_deps_pattern);

            if needs_update {
                let mut new_install_name = install_name.to_string();

                // Replace placeholder paths first
                if new_install_name.contains(crate::BUILD_PLACEHOLDER_PREFIX) {
                    new_install_name =
                        new_install_name.replace(crate::BUILD_PLACEHOLDER_PREFIX, actual_prefix);
                }

                // Replace build paths
                if new_install_name.contains(build_prefix) {
                    new_install_name = new_install_name.replace(build_prefix, actual_prefix);
                }

                // Also handle deps paths specifically
                let build_deps_pattern = format!("{}/deps", build_prefix);
                if new_install_name.contains(&build_deps_pattern) {
                    new_install_name = new_install_name.replace(&build_deps_pattern, actual_prefix);
                }

                // Update the install name
                let result = Command::new("install_name_tool")
                    .args(["-id", &new_install_name, &dylib_path.to_string_lossy()])
                    .output()
                    .await?;

                if result.status.success() {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Update dependency paths in a dylib in quality phase
async fn update_dylib_dependencies_quality(
    lines: &[&str],
    dylib_path: &Path,
    actual_prefix: &str,
    build_prefix: &str,
) -> Result<bool, Error> {
    use tokio::process::Command;

    let mut updated = false;

    for line in lines.iter().skip(1) {
        let trimmed = line.trim();
        if let Some(space_pos) = trimmed.find(" (") {
            let dep_path = &trimmed[..space_pos];

            // Check if it contains placeholder, build paths, or live/deps paths
            let live_deps_pattern = format!("{}/deps", actual_prefix);
            let needs_update = dep_path.contains(crate::BUILD_PLACEHOLDER_PREFIX)
                || dep_path.contains(build_prefix)
                || dep_path.contains(&live_deps_pattern);

            if needs_update {
                let mut new_dep_path = dep_path.to_string();

                // Replace placeholder paths first
                if new_dep_path.contains(crate::BUILD_PLACEHOLDER_PREFIX) {
                    new_dep_path =
                        new_dep_path.replace(crate::BUILD_PLACEHOLDER_PREFIX, actual_prefix);
                }

                // Replace build paths
                if new_dep_path.contains(build_prefix) {
                    new_dep_path = new_dep_path.replace(build_prefix, actual_prefix);
                }

                // Also handle deps paths specifically
                let build_deps_pattern = format!("{}/deps", build_prefix);
                if new_dep_path.contains(&build_deps_pattern) {
                    new_dep_path = new_dep_path.replace(&build_deps_pattern, actual_prefix);
                }

                // Handle case where deps might be under actual_prefix (like /opt/pm/live/deps)
                let live_deps_pattern = format!("{}/deps", actual_prefix);
                if new_dep_path.contains(&live_deps_pattern) {
                    new_dep_path = new_dep_path.replace(&live_deps_pattern, actual_prefix);
                }

                // Only update if the old and new paths are different
                if dep_path != new_dep_path {
                    // Update the dependency path
                    let result = Command::new("install_name_tool")
                        .args([
                            "-change",
                            dep_path,
                            &new_dep_path,
                            &dylib_path.to_string_lossy(),
                        ])
                        .output()
                        .await?;

                    if result.status.success() {
                        updated = true;
                    }
                }
            }
        }
    }
    Ok(updated)
}

/// Update RPATHs in a dylib in quality phase
async fn update_dylib_rpaths_quality(
    dylib_path: &Path,
    actual_prefix: &str,
    build_prefix: &str,
) -> Result<bool, Error> {
    use tokio::process::Command;

    let rpath_output = Command::new("otool")
        .args(["-l", &dylib_path.to_string_lossy()])
        .output()
        .await?;

    if !rpath_output.status.success() {
        return Ok(false);
    }

    let rpath_str = String::from_utf8_lossy(&rpath_output.stdout);
    let mut lines = rpath_str.lines();
    let mut updated = false;

    while let Some(line) = lines.next() {
        if line.contains("LC_RPATH") {
            // Skip the cmdsize line
            let _ = lines.next();
            // Get the path line
            if let Some(path_line) = lines.next() {
                if path_line.contains("path ") {
                    if let Some(path_start) = path_line.find("path ") {
                        let path_part = &path_line[path_start + 5..];
                        if let Some(space_pos) = path_part.find(" (") {
                            let rpath = &path_part[..space_pos];

                            // Check if it contains placeholder, build paths, or live/deps paths
                            let live_deps_pattern = format!("{}/deps", actual_prefix);
                            let needs_update = rpath.contains(crate::BUILD_PLACEHOLDER_PREFIX)
                                || rpath.contains(build_prefix)
                                || rpath.contains(&live_deps_pattern);

                            if needs_update {
                                let mut new_rpath = rpath.to_string();

                                // Replace placeholder paths first
                                if new_rpath.contains(crate::BUILD_PLACEHOLDER_PREFIX) {
                                    new_rpath = new_rpath
                                        .replace(crate::BUILD_PLACEHOLDER_PREFIX, actual_prefix);
                                }

                                // Replace build paths
                                if new_rpath.contains(build_prefix) {
                                    new_rpath = new_rpath.replace(build_prefix, actual_prefix);
                                }

                                // Also handle deps paths specifically
                                let build_deps_pattern = format!("{}/deps", build_prefix);
                                if new_rpath.contains(&build_deps_pattern) {
                                    new_rpath =
                                        new_rpath.replace(&build_deps_pattern, actual_prefix);
                                }

                                // First delete the old rpath
                                let _ = Command::new("install_name_tool")
                                    .args(["-delete_rpath", rpath, &dylib_path.to_string_lossy()])
                                    .output()
                                    .await;

                                // Then add the new one
                                let result = Command::new("install_name_tool")
                                    .args(["-add_rpath", &new_rpath, &dylib_path.to_string_lossy()])
                                    .output()
                                    .await;

                                if result.is_ok() {
                                    updated = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(updated)
}

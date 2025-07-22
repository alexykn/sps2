//! Fixes install‑name / `LC_RPATH` of Mach‑O dylibs & executables.

use crate::artifact_qa::{macho_utils, reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::{AppEvent, GeneralEvent, QaEvent};
use sps2_types::RpathStyle;

use ignore::WalkBuilder;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub struct RPathPatcher {
    style: RpathStyle,
}

impl RPathPatcher {
    /// Create a new `RPathPatcher` with the specified style
    ///
    /// The patcher will fix install names and RPATHs according to the given style.
    #[must_use]
    pub fn new(style: RpathStyle) -> Self {
        Self { style }
    }

    /// Check if a file is a dylib based on its name pattern
    /// Handles versioned dylibs like libfoo.1.dylib, libbar.2.3.4.dylib
    fn is_dylib(path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Check if the filename contains .dylib anywhere
            // This catches: libfoo.dylib, libfoo.1.dylib, libfoo.1.2.3.dylib
            name.contains(".dylib")
        } else {
            false
        }
    }

    /// Check if a file should be processed by `RPathPatcher`
    ///
    /// This includes dylibs, shared objects, and Mach-O executables. Returns true
    /// if the file needs RPATH or install name processing.
    #[must_use]
    pub fn should_process_file(path: &Path) -> bool {
        if !path.is_file() {
            return false;
        }

        // Check if it's a dylib (including versioned ones)
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.contains(".dylib") || name.contains(".so") {
                return true;
            }
        }

        // Use the shared MachO detection logic as a fallback
        // This will catch any Mach-O files we might have missed with filename patterns
        macho_utils::is_macho_file(path)
    }

    /// Get the install name of a Mach-O file using otool -D
    async fn get_install_name(path: &Path) -> Option<String> {
        let out = Command::new("otool")
            .args(["-D", &path.to_string_lossy()])
            .output()
            .await
            .ok()?;

        if !out.status.success() {
            return None;
        }

        let text = String::from_utf8_lossy(&out.stdout);
        // otool -D outputs:
        // /path/to/file:
        // install_name
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() >= 2 {
            Some(lines[1].trim().to_string())
        } else {
            None
        }
    }

    /// Check if an install name needs fixing based on the style
    fn needs_install_name_fix(&self, install_name: &str, _file_path: &Path) -> bool {
        match self.style {
            RpathStyle::Modern => {
                // Only fix install names that contain build paths
                // Do NOT fix @rpath/, @loader_path/, or @executable_path/ install names
                if install_name.starts_with("@rpath/")
                    || install_name.starts_with("@loader_path/")
                    || install_name.starts_with("@executable_path/")
                {
                    return false;
                }

                // Check if the install name contains a build path
                install_name.contains("/opt/pm/build") || install_name.contains("/private/")
            }
            RpathStyle::Absolute => {
                // Absolute style: Fix @rpath references AND build paths
                // Keep @loader_path/ and @executable_path/ as they're relative to the binary
                if install_name.starts_with("@loader_path/")
                    || install_name.starts_with("@executable_path/")
                {
                    return false;
                }

                // Fix @rpath references and build paths
                install_name.starts_with("@rpath/")
                    || install_name.contains("/opt/pm/build")
                    || install_name.contains("/private/")
            }
        }
    }

    /// Fix the install name of a dylib to use absolute path
    async fn fix_install_name(path: &Path, new_install_name: &str) -> Result<bool, String> {
        let output = Command::new("install_name_tool")
            .args(["-id", new_install_name, &path.to_string_lossy()])
            .output()
            .await
            .map_err(|e| format!("Failed to run install_name_tool: {e}"))?;

        if output.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check if this is a headerpad error
            if stderr.contains("larger updated load commands do not fit") {
                // This is a headerpad error - the binary needs more space in its header
                // We'll return a special error message that the caller can handle
                Err(format!("HEADERPAD_ERROR: {}", path.display()))
            } else {
                // Return generic error details for warning event
                Err(format!(
                    "install_name_tool failed on {}: {}",
                    path.display(),
                    stderr.trim()
                ))
            }
        }
    }

    /// Find all executables that link to a dylib and update their references
    async fn update_dylib_references(
        staging_dir: &Path,
        old_dylib_name: &str,
        new_dylib_path: &str,
    ) -> Result<Vec<PathBuf>, String> {
        let mut updated_files = Vec::new();
        let mut checked_files = HashSet::new();

        // Walk through all Mach-O files in the staging directory
        for entry in ignore::WalkBuilder::new(staging_dir)
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if !path.is_file() || !macho_utils::is_macho_file(&path) {
                continue;
            }

            // Skip if we've already checked this file
            if !checked_files.insert(path.clone()) {
                continue;
            }

            // Check if this file references the old dylib
            let output = Command::new("otool")
                .args(["-L", &path.to_string_lossy()])
                .output()
                .await
                .map_err(|e| format!("Failed to run otool: {e}"))?;

            if !output.status.success() {
                continue;
            }

            let deps = String::from_utf8_lossy(&output.stdout);
            if deps.contains(old_dylib_name) {
                // This file references our dylib - update the reference
                let change_output = Command::new("install_name_tool")
                    .args([
                        "-change",
                        old_dylib_name,
                        new_dylib_path,
                        &path.to_string_lossy(),
                    ])
                    .output()
                    .await
                    .map_err(|e| format!("Failed to run install_name_tool: {e}"))?;

                if change_output.status.success() {
                    updated_files.push(path);
                }
            }
        }

        Ok(updated_files)
    }

    /// Fix dependencies that use @rpath by converting them to absolute paths (Absolute style)
    async fn fix_rpath_dependencies(
        &self,
        path: &Path,
        lib_path: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let mut fixed_deps = Vec::new();

        // Skip if not using Absolute style
        if self.style != RpathStyle::Absolute {
            return Ok(fixed_deps);
        }

        // Get all dependencies using otool
        let output = Command::new("otool")
            .args(["-L", &path.to_string_lossy()])
            .output()
            .await
            .map_err(|e| format!("Failed to run otool: {e}"))?;

        if !output.status.success() {
            return Ok(fixed_deps);
        }

        let deps = String::from_utf8_lossy(&output.stdout);

        // Process each dependency line
        for line in deps.lines().skip(1) {
            // Skip the first line (file name)
            let dep = line.trim();
            if dep.starts_with("@rpath/") {
                // Extract the library name after @rpath/
                let lib_name = &dep[7..dep.find(" (").unwrap_or(dep.len())];
                let new_path = format!("{lib_path}/{lib_name}");

                // Update the dependency reference
                let change_output = Command::new("install_name_tool")
                    .args([
                        "-change",
                        dep.split_whitespace().next().unwrap(),
                        &new_path,
                        &path.to_string_lossy(),
                    ])
                    .output()
                    .await
                    .map_err(|e| format!("Failed to run install_name_tool: {e}"))?;

                if change_output.status.success() {
                    fixed_deps.push((dep.to_string(), new_path));
                }
            }
        }

        Ok(fixed_deps)
    }

    /// Process a single file for RPATH and install name fixes
    pub async fn process_file(
        &self,
        path: &Path,
        lib_path: &str,
        build_paths: &[String],
    ) -> (bool, bool, Vec<String>, Option<String>) {
        let path_s = path.to_string_lossy().into_owned();
        let mut bad_rpaths = Vec::new();
        let mut need_good = false;
        let mut install_name_was_fixed = false;

        // Use otool to inspect
        let out = Command::new("otool").args(["-l", &path_s]).output().await;
        let Ok(out) = out else {
            return (false, false, bad_rpaths, None);
        };
        if !out.status.success() {
            return (false, false, bad_rpaths, None);
        }
        let text = String::from_utf8_lossy(&out.stdout);

        // gather bad rpaths
        let mut lines = text.lines();
        while let Some(l) = lines.next() {
            if l.contains("LC_RPATH") {
                let _ = lines.next(); // skip cmdsize
                if let Some(p) = lines.next() {
                    if let Some(idx) = p.find("path ") {
                        let r = &p[idx + 5..p.find(" (").unwrap_or(p.len())];
                        if r == lib_path {
                            need_good = false;
                        } else if build_paths.iter().any(|bp| r.contains(bp)) {
                            // Flag any build paths as bad
                            bad_rpaths.push(r.to_owned());
                        }
                    }
                }
            } else if l.contains("@rpath/") {
                need_good = true;
            }
        }

        // Fix RPATHs
        // Only add RPATH for Modern style (for Absolute style, we convert @rpath to absolute paths)
        if need_good && self.style == RpathStyle::Modern {
            let _ = Command::new("install_name_tool")
                .args(["-add_rpath", lib_path, &path_s])
                .output()
                .await;
        }
        for bad in &bad_rpaths {
            let _ = Command::new("install_name_tool")
                .args(["-delete_rpath", bad, &path_s])
                .output()
                .await;
        }

        // Check and fix install names for dylibs
        if Self::is_dylib(path) {
            if let Some(install_name) = Self::get_install_name(path).await {
                if self.needs_install_name_fix(&install_name, path) {
                    // Fix the install name to absolute path
                    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let new_install_name = format!("{lib_path}/{file_name}");

                    match Self::fix_install_name(path, &new_install_name).await {
                        Ok(true) => install_name_was_fixed = true,
                        Ok(false) => {} // Should not happen with current implementation
                        Err(msg) => {
                            // Store error for reporting later
                            return (
                                need_good || !bad_rpaths.is_empty(),
                                false,
                                bad_rpaths,
                                Some(msg),
                            );
                        }
                    }
                }
            }
        }

        // Fix @rpath dependencies if using Absolute style
        if self.style == RpathStyle::Absolute {
            match self.fix_rpath_dependencies(path, lib_path).await {
                Ok(fixed_deps) => {
                    if !fixed_deps.is_empty() {
                        // Dependencies were fixed
                        // Note: We don't set a flag for this as it's part of Absolute-style processing
                    }
                }
                Err(msg) => {
                    // Store error for reporting later
                    return (
                        need_good || !bad_rpaths.is_empty(),
                        install_name_was_fixed,
                        bad_rpaths,
                        Some(msg),
                    );
                }
            }
        }

        (
            need_good || !bad_rpaths.is_empty(),
            install_name_was_fixed,
            bad_rpaths,
            None,
        )
    }

    /// Handle headerpad errors by updating references in dependent binaries
    async fn handle_headerpad_errors(
        patcher: &RPathPatcher,
        headerpad_errors: &[PathBuf],
        lib_path: &str,
        staging_dir: &Path,
        ctx: &BuildContext,
    ) -> (Vec<PathBuf>, Vec<String>) {
        let mut fixed_files = Vec::new();
        let mut warnings = Vec::new();

        if headerpad_errors.is_empty() {
            return (fixed_files, warnings);
        }

        crate::utils::events::send_event(
            ctx,
            AppEvent::Qa(QaEvent::FindingReported {
                check_type: "patcher".to_string(),
                severity: "warning".to_string(),
                message: format!(
                    "Found {} dylibs with headerpad errors. Attempting fallback strategy: updating references in dependent binaries",
                    headerpad_errors.len()
                ),
                file_path: None,
                line: None,
            }),
        );

        for dylib_path in headerpad_errors {
            if let Some(file_name) = dylib_path.file_name().and_then(|n| n.to_str()) {
                // Get the current install name that may need fixing
                if let Some(current_install_name) = Self::get_install_name(dylib_path).await {
                    if patcher.needs_install_name_fix(&current_install_name, dylib_path) {
                        // The desired new install name
                        let new_install_name = format!("{lib_path}/{file_name}");

                        // Update all binaries that reference this dylib
                        match Self::update_dylib_references(
                            staging_dir,
                            &current_install_name,
                            &new_install_name,
                        )
                        .await
                        {
                            Ok(updated_files) => {
                                if !updated_files.is_empty() {
                                    crate::utils::events::send_event(
                                        ctx,
                                        AppEvent::Qa(QaEvent::CheckCompleted {
                                            check_type: "patcher".to_string(),
                                            check_name: "rpath_headerpad_workaround".to_string(),
                                            findings_count: updated_files.len(),
                                            severity_counts: std::collections::HashMap::new(),
                                        }),
                                    );
                                    fixed_files.extend(updated_files);
                                }
                            }
                            Err(e) => {
                                warnings.push(format!(
                                    "Failed to update references for {file_name}: {e}"
                                ));
                            }
                        }
                    }
                }
            }
        }

        (fixed_files, warnings)
    }
}

impl RPathPatcher {
    /// Process all files that need rpath patching
    async fn process_files(
        &self,
        files: Vec<PathBuf>,
        lib_path: &str,
        build_paths: &[String],
        ctx: &BuildContext,
    ) -> (Vec<PathBuf>, usize, usize, Vec<PathBuf>, Vec<String>) {
        let mut changed = Vec::new();
        let mut install_name_fixes = 0;
        let mut rpath_fixes = 0;
        let mut warnings = Vec::new();
        let mut headerpad_errors = Vec::new();

        for path in files {
            let (rpath_changed, name_was_fixed, _, error_msg) =
                self.process_file(&path, lib_path, build_paths).await;

            if let Some(msg) = &error_msg {
                if msg.starts_with("HEADERPAD_ERROR:") {
                    headerpad_errors.push(path.clone());
                } else {
                    crate::utils::events::send_event(
                        ctx,
                        AppEvent::General(GeneralEvent::warning("Install name fix failed")),
                    );
                    warnings.push(format!("{}: install name fix failed", path.display()));
                }
            }

            if rpath_changed {
                rpath_fixes += 1;
            }
            if name_was_fixed {
                install_name_fixes += 1;
            }

            if rpath_changed || name_was_fixed {
                changed.push(path.clone());
            }
        }

        (
            changed,
            rpath_fixes,
            install_name_fixes,
            headerpad_errors,
            warnings,
        )
    }
}

impl crate::artifact_qa::traits::Action for RPathPatcher {
    const NAME: &'static str = "install_name_tool patcher";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        findings: Option<&crate::artifact_qa::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
        let patcher = Self::new(RpathStyle::Modern);
        let mut files_to_process = HashSet::new();

        // Collect files from validator findings
        if let Some(findings) = findings {
            let files_with_macho_issues = findings.get_files_with_macho_issues();
            for (path, _) in files_with_macho_issues {
                files_to_process.insert(path.to_path_buf());
            }
        }

        // Add files with @rpath references
        for entry in WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if Self::should_process_file(&path) {
                files_to_process.insert(path);
            }
        }

        let files: Vec<_> = files_to_process.into_iter().collect();
        let lib_path = "/opt/pm/live/lib";
        let build_paths = vec![
            "/opt/pm/build".to_string(),
            env.build_prefix().to_string_lossy().into_owned(),
            format!("{}/src", env.build_prefix().to_string_lossy()),
        ];

        // Process all files
        let (mut changed, rpath_fixes, install_name_fixes, headerpad_errors, mut warnings) =
            patcher
                .process_files(files, lib_path, &build_paths, ctx)
                .await;

        // Handle headerpad errors
        let (headerpad_fixed, headerpad_warnings) = Self::handle_headerpad_errors(
            &patcher,
            &headerpad_errors,
            lib_path,
            env.staging_dir(),
            ctx,
        )
        .await;
        changed.extend(headerpad_fixed);
        warnings.extend(headerpad_warnings);

        // Report results
        if !changed.is_empty() {
            let mut operations = Vec::new();
            if rpath_fixes > 0 {
                operations.push(format!(
                    "{} RPATH{}",
                    rpath_fixes,
                    if rpath_fixes > 1 { "s" } else { "" }
                ));
            }
            if install_name_fixes > 0 {
                operations.push(format!(
                    "{} install name{}",
                    install_name_fixes,
                    if install_name_fixes > 1 { "s" } else { "" }
                ));
            }

            crate::utils::events::send_event(
                ctx,
                AppEvent::Qa(QaEvent::CheckCompleted {
                    check_type: "patcher".to_string(),
                    check_name: "rpath".to_string(),
                    findings_count: changed.len(),
                    severity_counts: std::collections::HashMap::new(),
                }),
            );
        }

        Ok(Report {
            changed_files: changed,
            warnings,
            ..Default::default()
        })
    }
}
impl Patcher for RPathPatcher {}

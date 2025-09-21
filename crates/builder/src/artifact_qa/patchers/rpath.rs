//! Fixes install‑name / `LC_RPATH` of Mach‑O dylibs & executables.

use crate::artifact_qa::{macho_utils, reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::{AppEvent, EventSender, GeneralEvent};
use sps2_platform::{PlatformContext, PlatformManager};
use sps2_types::RpathStyle;

use ignore::WalkBuilder;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct RPathPatcher {
    style: RpathStyle,
    platform: &'static sps2_platform::Platform,
}

impl RPathPatcher {
    /// Create a new `RPathPatcher` with the specified style
    ///
    /// The patcher will fix install names and RPATHs according to the given style.
    #[must_use]
    pub fn new(style: RpathStyle) -> Self {
        Self {
            style,
            platform: PlatformManager::instance().platform(),
        }
    }

    /// Create a platform context for this patcher
    #[must_use]
    pub fn create_platform_context(&self, event_sender: Option<EventSender>) -> PlatformContext {
        self.platform.create_context(event_sender)
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

    /// Get the install name of a Mach-O file using platform abstraction
    async fn get_install_name(&self, ctx: &PlatformContext, path: &Path) -> Option<String> {
        (self.platform.binary().get_install_name(ctx, path).await).unwrap_or_default()
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
    async fn fix_install_name(
        &self,
        ctx: &PlatformContext,
        path: &Path,
        new_install_name: &str,
    ) -> Result<bool, String> {
        match self
            .platform
            .binary()
            .set_install_name(ctx, path, new_install_name)
            .await
        {
            Ok(()) => Ok(true),
            Err(platform_err) => {
                let err_msg = platform_err.to_string();
                // Check if this is a headerpad error
                if err_msg.contains("larger updated load commands do not fit") {
                    Err(format!("HEADERPAD_ERROR: {}", path.display()))
                } else {
                    Err(format!(
                        "install_name_tool failed on {}: {}",
                        path.display(),
                        err_msg
                    ))
                }
            }
        }
    }

    /// Find all executables that link to a dylib and update their references
    async fn update_dylib_references(
        &self,
        ctx: &PlatformContext,
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
            let Ok(deps) = self.platform.binary().get_dependencies(ctx, &path).await else {
                continue;
            };

            if deps.iter().any(|dep| dep.contains(old_dylib_name)) {
                // This file references our dylib - update the reference
                if let Ok(()) = self
                    .platform
                    .binary()
                    .change_dependency(ctx, &path, old_dylib_name, new_dylib_path)
                    .await
                {
                    updated_files.push(path);
                } else {
                    // Silently continue on error
                }
            }
        }

        Ok(updated_files)
    }

    /// Fix dependencies that use @rpath by converting them to absolute paths (Absolute style)
    async fn fix_rpath_dependencies(
        &self,
        ctx: &PlatformContext,
        path: &Path,
        lib_path: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let mut fixed_deps = Vec::new();

        // Skip if not using Absolute style
        if self.style != RpathStyle::Absolute {
            return Ok(fixed_deps);
        }

        // Get all dependencies using platform abstraction
        let Ok(deps) = self.platform.binary().get_dependencies(ctx, path).await else {
            return Ok(fixed_deps);
        };

        // Process each dependency
        for dep in deps {
            // Extract the library name after @rpath/
            if let Some(lib_name) = dep.strip_prefix("@rpath/") {
                let new_path = format!("{lib_path}/{lib_name}");

                // Update the dependency reference
                if let Ok(()) = self
                    .platform
                    .binary()
                    .change_dependency(ctx, path, &dep, &new_path)
                    .await
                {
                    fixed_deps.push((dep, new_path));
                } else {
                    // Continue on error - some dependencies might fail to update
                }
            }
        }

        Ok(fixed_deps)
    }

    /// Process a single file for RPATH and install name fixes
    pub async fn process_file(
        &self,
        ctx: &PlatformContext,
        path: &Path,
        lib_path: &str,
        build_paths: &[String],
    ) -> (bool, bool, Vec<String>, Option<String>) {
        let _path_s = path.to_string_lossy().into_owned();
        let mut bad_rpaths = Vec::new();
        let mut need_good = false;
        let mut install_name_was_fixed = false;

        // Get RPATH entries using platform abstraction
        let Ok(rpath_entries) = self.platform.binary().get_rpath_entries(ctx, path).await else {
            return (false, false, bad_rpaths, None);
        };

        // Check RPATH entries and gather bad ones
        let mut has_good_rpath = false;
        for rpath in &rpath_entries {
            if rpath == lib_path {
                has_good_rpath = true;
            } else if build_paths.iter().any(|bp| rpath.contains(bp)) {
                // Flag any build paths as bad
                bad_rpaths.push(rpath.clone());
            }
        }

        // Check if binary needs @rpath by examining dependencies
        let Ok(deps) = self.platform.binary().get_dependencies(ctx, path).await else {
            return (false, false, bad_rpaths, None);
        };

        for dep in &deps {
            if dep.contains("@rpath/") {
                need_good = true;
                break;
            }
        }

        // Fix RPATHs
        // Only add RPATH for Modern style (for Absolute style, we convert @rpath to absolute paths)
        if need_good && self.style == RpathStyle::Modern && !has_good_rpath {
            let _ = self.platform.binary().add_rpath(ctx, path, lib_path).await;
        }
        for bad in &bad_rpaths {
            let _ = self.platform.binary().delete_rpath(ctx, path, bad).await;
        }

        // Check and fix install names for dylibs
        if Self::is_dylib(path) {
            if let Some(install_name) = self.get_install_name(ctx, path).await {
                if self.needs_install_name_fix(&install_name, path) {
                    // Fix the install name to absolute path
                    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let new_install_name = format!("{lib_path}/{file_name}");

                    match self.fix_install_name(ctx, path, &new_install_name).await {
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
            match self.fix_rpath_dependencies(ctx, path, lib_path).await {
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
        platform_ctx: &PlatformContext,
        headerpad_errors: &[PathBuf],
        lib_path: &str,
        staging_dir: &Path,
        _build_ctx: &BuildContext,
    ) -> (Vec<PathBuf>, Vec<String>) {
        let mut fixed_files = Vec::new();
        let mut warnings = Vec::new();

        if headerpad_errors.is_empty() {
            return (fixed_files, warnings);
        }

        warnings.push(format!(
            "Found {} dylibs with headerpad errors; attempting fallback reference updates",
            headerpad_errors.len()
        ));

        for dylib_path in headerpad_errors {
            if let Some(file_name) = dylib_path.file_name().and_then(|n| n.to_str()) {
                // Get the current install name that may need fixing
                if let Some(current_install_name) =
                    patcher.get_install_name(platform_ctx, dylib_path).await
                {
                    if patcher.needs_install_name_fix(&current_install_name, dylib_path) {
                        // The desired new install name
                        let new_install_name = format!("{lib_path}/{file_name}");

                        // Update all binaries that reference this dylib
                        match patcher
                            .update_dylib_references(
                                platform_ctx,
                                staging_dir,
                                &current_install_name,
                                &new_install_name,
                            )
                            .await
                        {
                            Ok(updated_files) => {
                                if !updated_files.is_empty() {
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
        platform_ctx: &PlatformContext,
        build_ctx: &BuildContext,
    ) -> (Vec<PathBuf>, usize, usize, Vec<PathBuf>, Vec<String>) {
        let mut changed = Vec::new();
        let mut install_name_fixes = 0;
        let mut rpath_fixes = 0;
        let mut warnings = Vec::new();
        let mut headerpad_errors = Vec::new();

        for path in files {
            let (rpath_changed, name_was_fixed, _, error_msg) = self
                .process_file(platform_ctx, &path, lib_path, build_paths)
                .await;

            if let Some(msg) = &error_msg {
                if msg.starts_with("HEADERPAD_ERROR:") {
                    headerpad_errors.push(path.clone());
                } else {
                    crate::utils::events::send_event(
                        build_ctx,
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

        // Create platform context from build context
        let platform_ctx = patcher.platform.create_context(ctx.event_sender.clone());

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
        let lib_path = &format!("{}/lib", sps2_config::fixed_paths::LIVE_DIR);
        let build_paths = vec![
            "/opt/pm/build".to_string(),
            env.build_prefix().to_string_lossy().into_owned(),
            format!("{}/src", env.build_prefix().to_string_lossy()),
        ];

        // Process all files
        let (mut changed, rpath_fixes, install_name_fixes, headerpad_errors, mut warnings) =
            patcher
                .process_files(files, lib_path, &build_paths, &platform_ctx, ctx)
                .await;

        // Handle headerpad errors
        let (headerpad_fixed, headerpad_warnings) = Self::handle_headerpad_errors(
            &patcher,
            &platform_ctx,
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
                    "adjusted {} RPATH{}",
                    rpath_fixes,
                    if rpath_fixes > 1 { "s" } else { "" }
                ));
            }
            if install_name_fixes > 0 {
                operations.push(format!(
                    "updated {} install name{}",
                    install_name_fixes,
                    if install_name_fixes > 1 { "s" } else { "" }
                ));
            }
            if !operations.is_empty() {
                warnings.push(operations.join(", "));
            }
        }

        Ok(Report {
            changed_files: changed,
            warnings,
            ..Default::default()
        })
    }
}
impl Patcher for RPathPatcher {}

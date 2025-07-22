//! Ensures binaries and dynamic libraries have proper execute permissions
//!
//! This patcher comprehensively handles all types of executables:
//! - Dynamic libraries (.dylib, .so)
//! - All files in bin/, sbin/ directories
//! - Mach-O executables in libexec/
//! - Scripts with shebang lines (#!/bin/sh, etc.)
//! - Mach-O binaries anywhere in the package
//! - Common script files (.sh, .py, .pl, etc.)
//! - Build outputs (target/release/, .build/debug/, etc.)
//!
//! Some build systems don't set proper permissions, so this ensures
//! all executables are actually executable after installation.

use crate::artifact_qa::{macho_utils, reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::{AppEvent, QaEvent};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Default)]
pub struct PermissionsFixer {
    aggressive: bool,
}

impl PermissionsFixer {
    /// Create a new permissions fixer
    ///
    /// Set `aggressive` to true for more aggressive permission fixing (used with explicit calls).
    #[must_use]
    pub fn new(aggressive: bool) -> Self {
        Self { aggressive }
    }
}

impl PermissionsFixer {
    /// Check if a file is a dynamic library that needs execute permissions
    fn is_dynamic_library(path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Dynamic libraries need +x on macOS
            name.contains(".dylib") || name.contains(".so")
        } else {
            false
        }
    }

    /// Check if a file has a shebang (#!) indicating it's a script
    fn has_shebang(path: &Path) -> bool {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        if let Ok(file) = File::open(path) {
            let mut reader = BufReader::new(file);
            let mut first_line = String::new();
            if reader.read_line(&mut first_line).is_ok() && first_line.len() >= 2 {
                return first_line.starts_with("#!");
            }
        }
        false
    }

    /// Check if file needs execute permissions (conservative default mode)
    fn needs_execute_permission(path: &Path) -> bool {
        // Dynamic libraries need +x
        if Self::is_dynamic_library(path) {
            return true;
        }

        // Only check for Mach-O binaries by default
        macho_utils::is_macho_file(path)
    }

    /// Check if file needs execute permissions (aggressive mode for explicit `fix_permissions()` calls)
    ///
    /// This comprehensive check should be used to determine if a file needs execute permissions
    /// in aggressive mode, checking file location, type, and content.
    #[must_use]
    pub fn needs_execute_permission_aggressive(path: &Path) -> bool {
        // Dynamic libraries need +x
        if Self::is_dynamic_library(path) {
            return true;
        }

        // Check if file is in any common executable directory
        // We check parent directories to be more precise than string matching
        let mut current = path.parent();
        while let Some(parent) = current {
            if let Some(dir_name) = parent.file_name() {
                let dir_str = dir_name.to_string_lossy();

                // Standard executable directories
                if dir_str == "bin" || dir_str == "sbin" {
                    return true;
                }

                // libexec is special - only make Mach-O files executable
                if dir_str == "libexec" {
                    return macho_utils::is_macho_file(path);
                }

                // Cargo/Rust build directories
                if dir_str == "release" || dir_str == "debug" {
                    if let Some(grandparent) = parent.parent() {
                        if let Some(gp_name) = grandparent.file_name() {
                            if gp_name == ".build" || gp_name == "target" {
                                return true;
                            }
                        }
                    }
                }
            }
            current = parent.parent();
        }

        // Check for scripts with shebang (#!/bin/sh, #!/usr/bin/env python, etc.)
        if Self::has_shebang(path) {
            return true;
        }

        // Check for Mach-O binaries anywhere in the package
        if macho_utils::is_macho_file(path) {
            return true;
        }

        // Check for files with common executable extensions
        if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy();
            if ext_str == "sh"
                || ext_str == "bash"
                || ext_str == "zsh"
                || ext_str == "fish"
                || ext_str == "py"
                || ext_str == "pl"
                || ext_str == "rb"
                || ext_str == "lua"
            {
                return true;
            }
        }

        false
    }

    /// Fix permissions on a file if needed
    fn fix_permissions(path: &Path) -> Result<bool, std::io::Error> {
        let metadata = std::fs::metadata(path)?;
        let mut perms = metadata.permissions();
        let current_mode = perms.mode();

        // Check if any execute bit is already set
        if current_mode & 0o111 != 0 {
            return Ok(false); // Already has execute permissions
        }

        // Add execute permissions matching read permissions
        // If readable by owner, make executable by owner, etc.
        let new_mode = current_mode | ((current_mode & 0o444) >> 2); // Convert read bits to execute bits

        perms.set_mode(new_mode);
        std::fs::set_permissions(path, perms)?;

        Ok(true)
    }
}

impl crate::artifact_qa::traits::Action for PermissionsFixer {
    const NAME: &'static str = "Permissions fixer";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&crate::artifact_qa::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
        // Default instance uses conservative mode
        let fixer = Self::default();
        let mut fixed_count = 0;
        let mut errors = Vec::new();

        // Walk through all files in staging directory
        for entry in ignore::WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if !path.is_file() {
                continue;
            }

            // Use aggressive or conservative mode based on instance
            let needs_fix = if fixer.aggressive {
                Self::needs_execute_permission_aggressive(&path)
            } else {
                Self::needs_execute_permission(&path)
            };

            if !needs_fix {
                continue;
            }

            match Self::fix_permissions(&path) {
                Ok(true) => fixed_count += 1,
                Ok(false) => {} // Already had correct permissions
                Err(e) => {
                    errors.push(format!(
                        "Failed to fix permissions on {}: {}",
                        path.display(),
                        e
                    ));
                }
            }
        }

        if fixed_count > 0 {
            crate::utils::events::send_event(
                ctx,
                AppEvent::Qa(QaEvent::CheckCompleted {
                    check_type: "patcher".to_string(),
                    check_name: "permissions".to_string(),
                    findings_count: fixed_count,
                    severity_counts: std::collections::HashMap::new(),
                }),
            );
        }

        Ok(Report {
            errors,
            changed_files: vec![], // Permissions changes don't count as file content changes
            ..Default::default()
        })
    }
}

impl Patcher for PermissionsFixer {}

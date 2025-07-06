//! Build system-specific post-validation pipeline routing
//!
//! This module determines which validation pipeline to use based on the
//! build systems detected during the build process. Different build systems
//! have different requirements for post-validation to avoid breaking binaries.

use super::{PatcherAction, ValidatorAction};
use crate::artifact_qa::patchers::{
    binary_string::BinaryStringPatcher, codesigner::CodeSigner, headers::HeaderPatcher,
    la_cleaner::LaFileCleaner, object_cleaner::ObjectFileCleaner, pkgconfig::PkgConfigPatcher,
    placeholder::PlaceholderPatcher, python_isolation::PythonIsolationPatcher, rpath::RPathPatcher,
};
use crate::artifact_qa::scanners::{
    archive::ArchiveScanner, hardcoded::HardcodedScanner, macho::MachOScanner,
};
use sps2_types::{BuildSystemProfile, RpathStyle};
use std::collections::HashSet;
use std::hash::BuildHasher;

/// Determine the build system profile based on used build systems
pub fn determine_profile<S: BuildHasher>(
    used_build_systems: &HashSet<String, S>,
) -> BuildSystemProfile {
    // If empty, default to full validation
    if used_build_systems.is_empty() {
        return BuildSystemProfile::NativeFull;
    }

    // Check for specific build systems in priority order
    // Rust takes precedence - if Rust is used, we must use minimal validation
    if used_build_systems.contains("cargo") {
        return BuildSystemProfile::RustMinimal;
    }

    // Go is next priority
    if used_build_systems.contains("go") {
        return BuildSystemProfile::GoMedium;
    }

    // Script languages
    if used_build_systems.contains("python") || used_build_systems.contains("nodejs") {
        return BuildSystemProfile::ScriptLight;
    }

    // C/C++ build systems default to full validation
    BuildSystemProfile::NativeFull
}

/// Get validators for a specific build system profile
///
/// Returns the appropriate set of validators based on the build system profile.
/// Different profiles have different validation requirements.
#[must_use]
pub fn get_validators_for_profile(profile: BuildSystemProfile) -> Vec<ValidatorAction> {
    match profile {
        BuildSystemProfile::NativeFull => {
            // Full validation for C/C++ projects
            vec![
                ValidatorAction::HardcodedScanner(HardcodedScanner),
                ValidatorAction::MachOScanner(MachOScanner),
                ValidatorAction::ArchiveScanner(ArchiveScanner),
            ]
        }
        BuildSystemProfile::RustMinimal => {
            // Minimal validation for Rust to avoid breaking panic unwinding
            vec![
                // Skip HardcodedScanner - Rust binaries often have debug paths
                // Skip MachOScanner - Rust manages its own dylib paths
                // Skip ArchiveScanner for Rust
            ]
        }
        BuildSystemProfile::GoMedium => {
            // Medium validation for Go
            vec![
                ValidatorAction::HardcodedScanner(HardcodedScanner),
                ValidatorAction::MachOScanner(MachOScanner),
                // Skip ArchiveScanner for Go
            ]
        }
        BuildSystemProfile::ScriptLight => {
            // Light validation for scripting languages
            vec![
                ValidatorAction::HardcodedScanner(HardcodedScanner),
                // Skip binary scanners for script-based packages
            ]
        }
    }
}

/// Get patchers for a specific build system profile
///
/// Returns the appropriate set of patchers based on the build system profile.
/// The order of patchers is important - `CodeSigner` must always run last.
#[must_use]
pub fn get_patchers_for_profile(profile: BuildSystemProfile) -> Vec<PatcherAction> {
    match profile {
        BuildSystemProfile::NativeFull => {
            // Full patching pipeline for C/C++
            vec![
                // PermissionsFixer removed - only runs when explicitly called via fix_permissions()
                PatcherAction::PlaceholderPatcher(PlaceholderPatcher),
                PatcherAction::BinaryStringPatcher(BinaryStringPatcher),
                PatcherAction::RPathPatcher(RPathPatcher::new(RpathStyle::Modern)), // TODO: is this correct? I am confusion
                PatcherAction::HeaderPatcher(HeaderPatcher),
                PatcherAction::PkgConfigPatcher(PkgConfigPatcher),
                PatcherAction::LaFileCleaner(LaFileCleaner),
                PatcherAction::ObjectFileCleaner(ObjectFileCleaner),
                // CodeSigner MUST run last
                PatcherAction::CodeSigner(CodeSigner),
            ]
        }
        BuildSystemProfile::RustMinimal => {
            // Minimal patching for Rust - avoid binary patching and re-signing
            vec![
                // Skip everything - Rust sets permissions correctly
                // No permission fixing, no binary modifications, no code signing
            ]
        }
        BuildSystemProfile::GoMedium => {
            // Medium patching for Go
            vec![
                // PermissionsFixer removed - only runs when explicitly called
                PatcherAction::PlaceholderPatcher(PlaceholderPatcher),
                // Skip rpath patching (Go uses static linking mostly)
                // Minimal code signing if needed
                PatcherAction::CodeSigner(CodeSigner),
            ]
        }
        BuildSystemProfile::ScriptLight => {
            // Light patching for scripts
            vec![
                // PermissionsFixer removed - only runs when explicitly called
                PatcherAction::HeaderPatcher(HeaderPatcher),
                PatcherAction::PkgConfigPatcher(PkgConfigPatcher),
                PatcherAction::PythonIsolationPatcher(PythonIsolationPatcher),
                // Skip binary patchers for script packages
            ]
        }
    }
}

/// Get a descriptive name for the pipeline
///
/// Returns a human-readable name for the validation pipeline.
#[must_use]
pub fn get_pipeline_name(profile: BuildSystemProfile) -> &'static str {
    match profile {
        BuildSystemProfile::NativeFull => "Full C/C++ validation pipeline",
        BuildSystemProfile::RustMinimal => "Minimal Rust validation pipeline",
        BuildSystemProfile::GoMedium => "Medium Go validation pipeline",
        BuildSystemProfile::ScriptLight => "Light script validation pipeline",
    }
}

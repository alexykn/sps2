//! Public façade – `workflow.rs` calls only `run_quality_pipeline()`.

pub mod diagnostics;
pub mod macho_utils;
pub mod patchers;
pub mod reports;
pub mod router;
pub mod scanners;
pub mod traits;

use crate::{utils::events::send_event, BuildContext, BuildEnvironment};
use diagnostics::DiagnosticCollector;
use reports::{MergedReport, Report};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use traits::Action;

/// Enum for all validators
pub enum ValidatorAction {
    HardcodedScanner(scanners::hardcoded::HardcodedScanner),
    MachOScanner(scanners::macho::MachOScanner),
    ArchiveScanner(scanners::archive::ArchiveScanner),
}

/// Enum for all patchers
pub enum PatcherAction {
    PermissionsFixer(patchers::permissions::PermissionsFixer),
    PlaceholderPatcher(patchers::placeholder::PlaceholderPatcher),
    RPathPatcher(patchers::rpath::RPathPatcher),
    HeaderPatcher(patchers::headers::HeaderPatcher),
    PkgConfigPatcher(patchers::pkgconfig::PkgConfigPatcher),
    BinaryStringPatcher(patchers::binary_string::BinaryStringPatcher),
    LaFileCleaner(patchers::la_cleaner::LaFileCleaner),
    ObjectFileCleaner(patchers::object_cleaner::ObjectFileCleaner),
    CodeSigner(patchers::codesigner::CodeSigner),
}

impl ValidatorAction {
    fn name(&self) -> &'static str {
        match self {
            Self::HardcodedScanner(_) => scanners::hardcoded::HardcodedScanner::NAME,
            Self::MachOScanner(_) => scanners::macho::MachOScanner::NAME,
            Self::ArchiveScanner(_) => scanners::archive::ArchiveScanner::NAME,
        }
    }

    async fn run(
        &self,
        ctx: &BuildContext,
        env: &BuildEnvironment,
        findings: Option<&DiagnosticCollector>,
    ) -> Result<Report, Error> {
        match self {
            Self::HardcodedScanner(_) => {
                scanners::hardcoded::HardcodedScanner::run(ctx, env, findings).await
            }
            Self::MachOScanner(_) => scanners::macho::MachOScanner::run(ctx, env, findings).await,
            Self::ArchiveScanner(_) => {
                scanners::archive::ArchiveScanner::run(ctx, env, findings).await
            }
        }
    }
}

impl PatcherAction {
    fn name(&self) -> &'static str {
        match self {
            Self::PermissionsFixer(_) => patchers::permissions::PermissionsFixer::NAME,
            Self::PlaceholderPatcher(_) => patchers::placeholder::PlaceholderPatcher::NAME,
            Self::RPathPatcher(_) => patchers::rpath::RPathPatcher::NAME,
            Self::HeaderPatcher(_) => patchers::headers::HeaderPatcher::NAME,
            Self::PkgConfigPatcher(_) => patchers::pkgconfig::PkgConfigPatcher::NAME,
            Self::BinaryStringPatcher(_) => patchers::binary_string::BinaryStringPatcher::NAME,
            Self::LaFileCleaner(_) => patchers::la_cleaner::LaFileCleaner::NAME,
            Self::ObjectFileCleaner(_) => patchers::object_cleaner::ObjectFileCleaner::NAME,
            Self::CodeSigner(_) => patchers::codesigner::CodeSigner::NAME,
        }
    }

    async fn run(
        &self,
        ctx: &BuildContext,
        env: &BuildEnvironment,
        findings: Option<&DiagnosticCollector>,
    ) -> Result<Report, Error> {
        match self {
            Self::PermissionsFixer(_) => {
                patchers::permissions::PermissionsFixer::run(ctx, env, findings).await
            }
            Self::PlaceholderPatcher(_) => {
                patchers::placeholder::PlaceholderPatcher::run(ctx, env, findings).await
            }
            Self::RPathPatcher(_) => patchers::rpath::RPathPatcher::run(ctx, env, findings).await,
            Self::HeaderPatcher(_) => {
                patchers::headers::HeaderPatcher::run(ctx, env, findings).await
            }
            Self::PkgConfigPatcher(_) => {
                patchers::pkgconfig::PkgConfigPatcher::run(ctx, env, findings).await
            }
            Self::BinaryStringPatcher(_) => {
                patchers::binary_string::BinaryStringPatcher::run(ctx, env, findings).await
            }
            Self::LaFileCleaner(_) => {
                patchers::la_cleaner::LaFileCleaner::run(ctx, env, findings).await
            }
            Self::ObjectFileCleaner(_) => {
                patchers::object_cleaner::ObjectFileCleaner::run(ctx, env, findings).await
            }
            Self::CodeSigner(_) => patchers::codesigner::CodeSigner::run(ctx, env, findings).await,
        }
    }
}

/// Replace the former `run_quality_checks()`
///
/// * V1 – pre‑validation
/// * P – patch tree in‑place
/// * V2 – must be clean, else the build fails
pub async fn run_quality_pipeline(ctx: &BuildContext, env: &BuildEnvironment) -> Result<(), Error> {
    // Determine which pipeline to use based on build systems
    let used_build_systems = env.used_build_systems();
    let profile = router::determine_profile(used_build_systems);

    // Log which pipeline we're using
    send_event(
        ctx,
        Event::DebugLog {
            message: format!(
                "Using {} for build systems: {:?}",
                router::get_pipeline_name(profile),
                used_build_systems
            ),
            context: std::collections::HashMap::new(),
        },
    );
    // ----------------    PHASE 1  -----------------
    let mut pre = run_validators(
        ctx,
        env,
        router::get_validators_for_profile(profile),
        false, // Don't allow early break - run all validators
    )
    .await?;

    // Extract findings from Phase 1 validators to pass to patchers
    let validator_findings = pre.take_findings();

    // ----------------    PHASE 2  -----------------
    run_patchers(
        ctx,
        env,
        validator_findings,
        router::get_patchers_for_profile(profile),
    )
    .await?;

    // ----------------    PHASE 3  -----------------
    let post = run_validators(
        ctx,
        env,
        router::get_validators_for_profile(profile),
        true, // Allow early break in final validation
    )
    .await?;

    if post.is_fatal() {
        return Err(BuildError::Failed {
            message: post.render("Relocatability check failed"),
        }
        .into());
    } else if !pre.is_fatal() && !post.is_fatal() {
        // Emit a short success note
        send_event(
            ctx,
            Event::OperationCompleted {
                operation: "Post‑build validation".into(),
                success: true,
            },
        );
    }
    Ok(())
}

/// Utility that runs validators and merges their reports.
async fn run_validators(
    ctx: &BuildContext,
    env: &BuildEnvironment,
    actions: Vec<ValidatorAction>,
    allow_early_break: bool,
) -> Result<MergedReport, Error> {
    let mut merged = MergedReport::default();

    for action in &actions {
        let action_name = action.name();
        send_event(
            ctx,
            Event::OperationStarted {
                operation: action_name.into(),
            },
        );
        let rep = action.run(ctx, env, None).await?;
        send_event(
            ctx,
            Event::OperationCompleted {
                operation: action_name.into(),
                success: !rep.is_fatal(),
            },
        );
        merged.absorb(rep);
        if allow_early_break && merged.is_fatal() {
            break; // short‑circuit early (saves time)
        }
    }
    Ok(merged)
}

/// Utility that runs patchers and merges their reports.
async fn run_patchers(
    ctx: &BuildContext,
    env: &BuildEnvironment,
    validator_findings: Option<DiagnosticCollector>,
    actions: Vec<PatcherAction>,
) -> Result<MergedReport, Error> {
    let mut merged = MergedReport::default();

    for action in &actions {
        let action_name = action.name();
        send_event(
            ctx,
            Event::OperationStarted {
                operation: action_name.into(),
            },
        );
        let rep = action.run(ctx, env, validator_findings.as_ref()).await?;
        send_event(
            ctx,
            Event::OperationCompleted {
                operation: action_name.into(),
                success: !rep.is_fatal(),
            },
        );
        merged.absorb(rep);
        if merged.is_fatal() {
            break; // short‑circuit early (saves time)
        }
    }
    Ok(merged)
}

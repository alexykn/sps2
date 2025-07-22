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
use sps2_events::{AppEvent, GeneralEvent, QaEvent};
use traits::Action;

/// Enum for all validators
pub enum ValidatorAction {
    HardcodedScanner(scanners::hardcoded::HardcodedScanner),
    MachOScanner(scanners::macho::MachOScanner),
    ArchiveScanner(scanners::archive::ArchiveScanner),
    StagingScanner(scanners::staging::StagingScanner),
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
    PythonIsolationPatcher(patchers::python_isolation::PythonIsolationPatcher),
    CodeSigner(patchers::codesigner::CodeSigner),
}

impl ValidatorAction {
    fn name(&self) -> &'static str {
        match self {
            Self::HardcodedScanner(_) => scanners::hardcoded::HardcodedScanner::NAME,
            Self::MachOScanner(_) => scanners::macho::MachOScanner::NAME,
            Self::ArchiveScanner(_) => scanners::archive::ArchiveScanner::NAME,
            Self::StagingScanner(_) => scanners::staging::StagingScanner::NAME,
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
            Self::StagingScanner(_) => {
                scanners::staging::StagingScanner::run(ctx, env, findings).await
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
            Self::PythonIsolationPatcher(_) => {
                patchers::python_isolation::PythonIsolationPatcher::NAME
            }
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
            Self::PythonIsolationPatcher(_) => {
                patchers::python_isolation::PythonIsolationPatcher::run(ctx, env, findings).await
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
///
/// # Errors
///
/// Returns an error if:
/// - Any scanner detects critical issues
/// - Failed to apply patches during the patching phase
/// - I/O errors occur during file analysis
/// - The final validation phase fails (V2 phase)
///
/// # Panics
///
/// This function will panic if `qa_override` results in a profile selection that returns `None`
/// from `determine_profile_with_override` but is not the `Skip` variant (this should not happen
/// in normal operation).
pub async fn run_quality_pipeline(
    ctx: &BuildContext,
    env: &BuildEnvironment,
    qa_override: Option<sps2_types::QaPipelineOverride>,
) -> Result<(), Error> {
    // Determine which pipeline to use based on build systems and override
    let used_build_systems = env.used_build_systems();
    let profile_opt = router::determine_profile_with_override(used_build_systems, qa_override);

    // Check if QA is skipped entirely
    if profile_opt.is_none() {
        send_event(
            ctx,
            AppEvent::General(GeneralEvent::debug("Artifact QA pipeline completed")),
        );
        return Ok(());
    }
    let profile = profile_opt.unwrap();
    // ----------------    PHASE1  -----------------
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
            AppEvent::General(GeneralEvent::OperationCompleted {
                operation: "Post‑build validation".into(),
                success: true,
            }),
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
            AppEvent::Qa(QaEvent::CheckStarted {
                check_type: "validator".to_string(),
                check_name: action_name.to_string(),
            }),
        );
        let rep = action.run(ctx, env, None).await?;
        send_event(
            ctx,
            AppEvent::Qa(QaEvent::CheckCompleted {
                check_type: "validator".to_string(),
                check_name: action_name.to_string(),
                findings_count: rep.findings.as_ref().map_or(0, |f| f.count()),
                severity_counts: std::collections::HashMap::new(),
            }),
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
            AppEvent::Qa(QaEvent::CheckStarted {
                check_type: "patcher".to_string(),
                check_name: action_name.to_string(),
            }),
        );
        let rep = action.run(ctx, env, validator_findings.as_ref()).await?;
        send_event(
            ctx,
            AppEvent::Qa(QaEvent::CheckCompleted {
                check_type: "patcher".to_string(),
                check_name: action_name.to_string(),
                findings_count: rep.findings.as_ref().map_or(0, |f| f.count()),
                severity_counts: std::collections::HashMap::new(),
            }),
        );
        merged.absorb(rep);
        if merged.is_fatal() {
            break; // short‑circuit early (saves time)
        }
    }
    Ok(merged)
}

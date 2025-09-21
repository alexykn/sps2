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
use sps2_events::{
    events::{QaCheckStatus, QaCheckSummary, QaFinding, QaLevel, QaSeverity, QaTarget},
    AppEvent, FailureContext, GeneralEvent, QaEvent,
};
use sps2_types::BuildSystemProfile;
use std::convert::TryFrom;
use std::time::{Duration, Instant};
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
    PythonBytecodeCleanupPatcher(patchers::python_bytecode_cleanup::PythonBytecodeCleanupPatcher),
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
            Self::PythonBytecodeCleanupPatcher(_) => {
                patchers::python_bytecode_cleanup::PythonBytecodeCleanupPatcher::NAME
            }
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
            Self::PythonBytecodeCleanupPatcher(_) => {
                patchers::python_bytecode_cleanup::PythonBytecodeCleanupPatcher::run(
                    ctx, env, findings,
                )
                .await
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
    let pipeline_start = Instant::now();
    let mut stats = QaStats::default();
    let target = qa_target(ctx);

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
    let qa_level = qa_level_for_profile(profile);

    send_event(
        ctx,
        AppEvent::Qa(QaEvent::PipelineStarted {
            target: target.clone(),
            level: qa_level,
        }),
    );

    // ----------------    PHASE 1  -----------------
    let mut pre = match run_validators(
        ctx,
        env,
        router::get_validators_for_profile(profile),
        false, // Don't allow early break - run all validators
        &target,
        &mut stats,
    )
    .await
    {
        Ok(report) => report,
        Err(err) => {
            emit_pipeline_failed(ctx, &target, &err);
            return Err(err);
        }
    };

    // Extract findings from Phase 1 validators to pass to patchers
    let validator_findings = pre.take_findings();

    // ----------------    PHASE 2  -----------------
    if let Err(err) = run_patchers(
        ctx,
        env,
        validator_findings,
        router::get_patchers_for_profile(profile),
        &target,
        &mut stats,
    )
    .await
    {
        emit_pipeline_failed(ctx, &target, &err);
        return Err(err);
    }

    // ----------------    PHASE 3  -----------------
    let post = match run_validators(
        ctx,
        env,
        router::get_validators_for_profile(profile),
        true, // Allow early break in final validation
        &target,
        &mut stats,
    )
    .await
    {
        Ok(report) => report,
        Err(err) => {
            emit_pipeline_failed(ctx, &target, &err);
            return Err(err);
        }
    };

    if post.is_fatal() {
        let failure_error: Error = BuildError::Failed {
            message: post.render("Relocatability check failed"),
        }
        .into();
        emit_pipeline_failed(ctx, &target, &failure_error);
        return Err(failure_error);
    } else if !pre.is_fatal() && !post.is_fatal() {
        send_event(
            ctx,
            AppEvent::General(GeneralEvent::OperationCompleted {
                operation: "Post‑build validation".into(),
                success: true,
            }),
        );
    }

    let duration_ms = u64::try_from(pipeline_start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let total_checks = stats.total;
    let failed_checks = stats.failed;
    let passed_checks = total_checks.saturating_sub(failed_checks);

    send_event(
        ctx,
        AppEvent::Qa(QaEvent::PipelineCompleted {
            target,
            total_checks,
            passed: passed_checks,
            failed: failed_checks,
            duration_ms,
        }),
    );

    Ok(())
}

/// Utility that runs validators and merges their reports.
async fn run_validators(
    ctx: &BuildContext,
    env: &BuildEnvironment,
    actions: Vec<ValidatorAction>,
    allow_early_break: bool,
    target: &QaTarget,
    stats: &mut QaStats,
) -> Result<MergedReport, Error> {
    let mut merged = MergedReport::default();

    for action in &actions {
        let action_name = action.name();
        let check_start = Instant::now();
        let rep = action.run(ctx, env, None).await?;
        emit_qa_check(
            ctx,
            target,
            "validator",
            action_name,
            &rep,
            check_start.elapsed(),
            stats,
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
    target: &QaTarget,
    stats: &mut QaStats,
) -> Result<MergedReport, Error> {
    let mut merged = MergedReport::default();

    for action in &actions {
        let action_name = action.name();
        let check_start = Instant::now();
        let rep = action.run(ctx, env, validator_findings.as_ref()).await?;
        emit_qa_check(
            ctx,
            target,
            "patcher",
            action_name,
            &rep,
            check_start.elapsed(),
            stats,
        );
        merged.absorb(rep);
        if merged.is_fatal() {
            break; // short‑circuit early (saves time)
        }
    }
    Ok(merged)
}
#[derive(Default)]
struct QaStats {
    total: usize,
    failed: usize,
}

fn qa_target(ctx: &BuildContext) -> QaTarget {
    QaTarget {
        package: ctx.name.clone(),
        version: ctx.version.clone(),
    }
}

fn qa_level_for_profile(profile: BuildSystemProfile) -> QaLevel {
    match profile {
        BuildSystemProfile::NativeFull => QaLevel::Strict,
        BuildSystemProfile::GoMedium => QaLevel::Standard,
        BuildSystemProfile::ScriptLight | BuildSystemProfile::RustMinimal => QaLevel::Fast,
    }
}

fn qa_findings_from_report(report: &Report) -> Vec<QaFinding> {
    let mut findings = Vec::new();

    for message in &report.errors {
        findings.push(QaFinding {
            severity: QaSeverity::Error,
            message: message.clone(),
            file: None,
            line: None,
        });
    }

    for message in &report.warnings {
        findings.push(QaFinding {
            severity: QaSeverity::Warning,
            message: message.clone(),
            file: None,
            line: None,
        });
    }

    if let Some(diags) = &report.findings {
        for finding in diags.findings() {
            findings.push(QaFinding {
                severity: QaSeverity::Warning,
                message: finding.issue_type.description(),
                file: Some(finding.file_path.clone()),
                line: None,
            });
        }
    }

    findings
}

fn build_check_summary(
    category: &str,
    name: &str,
    report: &Report,
    duration: Duration,
) -> QaCheckSummary {
    QaCheckSummary {
        name: name.to_string(),
        category: category.to_string(),
        status: if report.is_fatal() {
            QaCheckStatus::Failed
        } else {
            QaCheckStatus::Passed
        },
        duration_ms: Some(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
        findings: qa_findings_from_report(report),
    }
}

fn emit_qa_check(
    ctx: &BuildContext,
    target: &QaTarget,
    category: &str,
    action_name: &str,
    report: &Report,
    duration: Duration,
    stats: &mut QaStats,
) {
    let summary = build_check_summary(category, action_name, report, duration);
    stats.total += 1;
    if matches!(summary.status, QaCheckStatus::Failed) {
        stats.failed += 1;
    }

    send_event(
        ctx,
        AppEvent::Qa(QaEvent::CheckEvaluated {
            target: target.clone(),
            summary,
        }),
    );
}

fn emit_pipeline_failed(ctx: &BuildContext, target: &QaTarget, error: &Error) {
    send_event(
        ctx,
        AppEvent::Qa(QaEvent::PipelineFailed {
            target: target.clone(),
            failure: FailureContext::from_error(error),
        }),
    );
}

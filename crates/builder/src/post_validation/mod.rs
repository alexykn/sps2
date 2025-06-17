//! Public façade – `workflow.rs` calls only `run_quality_pipeline()`.

pub mod diagnostics;
pub mod patchers;
pub mod reports;
pub mod scanners;
pub mod traits;

use crate::{events::send_event, BuildContext, BuildEnvironment};
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
    PlaceholderPatcher(patchers::placeholder::PlaceholderPatcher),
    RPathPatcher(patchers::rpath::RPathPatcher),
    HeaderPatcher(patchers::headers::HeaderPatcher),
    PkgConfigPatcher(patchers::pkgconfig::PkgConfigPatcher),
    BinaryStringPatcher(patchers::binary_string::BinaryStringPatcher),
    LaFileCleaner(patchers::la_cleaner::LaFileCleaner),
    ObjectFileCleaner(patchers::object_cleaner::ObjectFileCleaner),
}

impl ValidatorAction {
    fn name(&self) -> &'static str {
        match self {
            Self::HardcodedScanner(_) => scanners::hardcoded::HardcodedScanner::NAME,
            Self::MachOScanner(_) => scanners::macho::MachOScanner::NAME,
            Self::ArchiveScanner(_) => scanners::archive::ArchiveScanner::NAME,
        }
    }

    async fn run(&self, ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        match self {
            Self::HardcodedScanner(_) => scanners::hardcoded::HardcodedScanner::run(ctx, env).await,
            Self::MachOScanner(_) => scanners::macho::MachOScanner::run(ctx, env).await,
            Self::ArchiveScanner(_) => scanners::archive::ArchiveScanner::run(ctx, env).await,
        }
    }
}

impl PatcherAction {
    fn name(&self) -> &'static str {
        match self {
            Self::PlaceholderPatcher(_) => patchers::placeholder::PlaceholderPatcher::NAME,
            Self::RPathPatcher(_) => patchers::rpath::RPathPatcher::NAME,
            Self::HeaderPatcher(_) => patchers::headers::HeaderPatcher::NAME,
            Self::PkgConfigPatcher(_) => patchers::pkgconfig::PkgConfigPatcher::NAME,
            Self::BinaryStringPatcher(_) => patchers::binary_string::BinaryStringPatcher::NAME,
            Self::LaFileCleaner(_) => patchers::la_cleaner::LaFileCleaner::NAME,
            Self::ObjectFileCleaner(_) => patchers::object_cleaner::ObjectFileCleaner::NAME,
        }
    }

    async fn run(&self, ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        match self {
            Self::PlaceholderPatcher(_) => {
                patchers::placeholder::PlaceholderPatcher::run(ctx, env).await
            }
            Self::RPathPatcher(_) => patchers::rpath::RPathPatcher::run(ctx, env).await,
            Self::HeaderPatcher(_) => patchers::headers::HeaderPatcher::run(ctx, env).await,
            Self::PkgConfigPatcher(_) => patchers::pkgconfig::PkgConfigPatcher::run(ctx, env).await,
            Self::BinaryStringPatcher(_) => {
                patchers::binary_string::BinaryStringPatcher::run(ctx, env).await
            }
            Self::LaFileCleaner(_) => patchers::la_cleaner::LaFileCleaner::run(ctx, env).await,
            Self::ObjectFileCleaner(_) => {
                patchers::object_cleaner::ObjectFileCleaner::run(ctx, env).await
            }
        }
    }
}

/// Replace the former `run_quality_checks()`
///
/// * V1 – pre‑validation
/// * P – patch tree in‑place
/// * V2 – must be clean, else the build fails
pub async fn run_quality_pipeline(ctx: &BuildContext, env: &BuildEnvironment) -> Result<(), Error> {
    // ----------------    PHASE 1  -----------------
    let pre = run_validators(
        ctx,
        env,
        vec![
            ValidatorAction::HardcodedScanner(scanners::hardcoded::HardcodedScanner),
            ValidatorAction::MachOScanner(scanners::macho::MachOScanner),
            ValidatorAction::ArchiveScanner(scanners::archive::ArchiveScanner),
        ],
    )
    .await?;

    // ----------------    PHASE 2  -----------------
    run_patchers(
        ctx,
        env,
        vec![
            PatcherAction::PlaceholderPatcher(patchers::placeholder::PlaceholderPatcher),
            PatcherAction::BinaryStringPatcher(patchers::binary_string::BinaryStringPatcher),
            PatcherAction::RPathPatcher(patchers::rpath::RPathPatcher),
            PatcherAction::HeaderPatcher(patchers::headers::HeaderPatcher),
            PatcherAction::PkgConfigPatcher(patchers::pkgconfig::PkgConfigPatcher),
            PatcherAction::LaFileCleaner(patchers::la_cleaner::LaFileCleaner),
            PatcherAction::ObjectFileCleaner(patchers::object_cleaner::ObjectFileCleaner),
        ],
    )
    .await?;

    // ----------------    PHASE 3  -----------------
    let post = run_validators(
        ctx,
        env,
        vec![
            ValidatorAction::HardcodedScanner(scanners::hardcoded::HardcodedScanner),
            ValidatorAction::MachOScanner(scanners::macho::MachOScanner),
            ValidatorAction::ArchiveScanner(scanners::archive::ArchiveScanner),
        ],
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
        let rep = action.run(ctx, env).await?;
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

/// Utility that runs patchers and merges their reports.
async fn run_patchers(
    ctx: &BuildContext,
    env: &BuildEnvironment,
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
        let rep = action.run(ctx, env).await?;
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

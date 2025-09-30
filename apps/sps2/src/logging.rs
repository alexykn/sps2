//! Structured logging integration for events
//!
//! This module provides structured logging capabilities that integrate with the
//! tracing ecosystem, converting domain-specific events into appropriate log
//! records with structured fields.

use sps2_events::{AppEvent, EventMessage};
use tracing::{debug, error, info, trace, warn};

/// Log an AppEvent using the tracing infrastructure with structured fields
///
/// This function takes an AppEvent and logs it at the appropriate level with
/// structured fields that can be consumed by observability tools.
pub fn log_event_with_tracing(message: &EventMessage) {
    let event = &message.event;
    let meta = &message.meta;
    let level = meta.tracing_level();
    // Extract structured fields based on event type
    match event {
        // Download domain events
        AppEvent::Lifecycle(sps2_events::events::LifecycleEvent::Download {
            stage,
            context,
            failure,
        }) => {
            use sps2_events::events::LifecycleStage;
            match stage {
                LifecycleStage::Started => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %context.url,
                        package = ?context.package,
                        total_bytes = ?context.total_bytes,
                        "Download started"
                    );
                }
                LifecycleStage::Completed => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %context.url,
                        package = ?context.package,
                        bytes_downloaded = ?context.bytes_downloaded,
                        "Download completed"
                    );
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        error!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            url = %context.url,
                            package = ?context.package,
                            retryable = failure_ctx.retryable,
                            code = ?failure_ctx.code,
                            message = %failure_ctx.message,
                            hint = ?failure_ctx.hint,
                            "Download failed"
                        );
                    }
                }
            }
        }

        AppEvent::Build(build_event) => {
            use sps2_events::{BuildDiagnostic, BuildEvent, LogStream, PhaseStatus};
            match build_event {
                BuildEvent::Started { session, target } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %target.package,
                        version = %target.version,
                        system = ?session.system,
                        cache_enabled = session.cache_enabled,
                        "Build started"
                    );
                }
                BuildEvent::Completed {
                    target,
                    artifacts,
                    duration_ms,
                    ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %target.package,
                        version = %target.version,
                        artifacts = artifacts.len(),
                        duration_ms,
                        "Build completed"
                    );
                }
                BuildEvent::Failed {
                    target,
                    failure,
                    phase,
                    command,
                    ..
                } => {
                    if failure.retryable {
                        warn!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            package = %target.package,
                            version = %target.version,
                            phase = ?phase,
                            command = ?command.as_ref().map(|c| &c.command),
                            retryable = failure.retryable,
                            code = ?failure.code,
                            message = %failure.message,
                            hint = ?failure.hint,
                            "Build failed",
                        );
                    } else {
                        error!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            package = %target.package,
                            version = %target.version,
                            phase = ?phase,
                            command = ?command.as_ref().map(|c| &c.command),
                            retryable = failure.retryable,
                            code = ?failure.code,
                            message = %failure.message,
                            hint = ?failure.hint,
                            "Build failed",
                        );
                    }
                }
                BuildEvent::PhaseStatus { phase, status, .. } => match status {
                    PhaseStatus::Started => {
                        info!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            phase = ?phase,
                            "Build phase started"
                        );
                    }
                    PhaseStatus::Completed { duration_ms } => {
                        info!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            phase = ?phase,
                            duration_ms,
                            "Build phase completed"
                        );
                    }
                },
                BuildEvent::Diagnostic(diag) => match diag {
                    BuildDiagnostic::Warning {
                        message,
                        source: warn_source,
                        ..
                    } => {
                        warn!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            diagnostic_source = ?warn_source,
                            message = %message,
                            "Build warning",
                        );
                    }
                    BuildDiagnostic::LogChunk { stream, text, .. } => match stream {
                        LogStream::Stdout => {
                            debug!(
                                source = meta.source.as_str(),
                                event_id = %meta.event_id,
                                correlation = ?meta.correlation_id,
                                stream = "stdout",
                                text = %text,
                                "Build output"
                            );
                        }
                        LogStream::Stderr => {
                            debug!(
                                source = meta.source.as_str(),
                                event_id = %meta.event_id,
                                correlation = ?meta.correlation_id,
                                stream = "stderr",
                                text = %text,
                                "Build output"
                            );
                        }
                    },
                    BuildDiagnostic::CachePruned {
                        removed_items,
                        freed_bytes,
                    } => {
                        debug!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            removed_items,
                            freed_bytes,
                            "Build cache pruned"
                        );
                    }
                },
            }
        }

        AppEvent::Guard(guard_event) => {
            use sps2_events::{GuardEvent, GuardScope, GuardSeverity};

            fn scope_label(scope: &GuardScope) -> String {
                match scope {
                    GuardScope::System => "system".to_string(),
                    GuardScope::Package { name, version } => version
                        .as_ref()
                        .map(|v| format!("{name}:{v}"))
                        .unwrap_or_else(|| name.clone()),
                    GuardScope::Path { path } => path.clone(),
                    GuardScope::State { id } => format!("state {id}"),
                    GuardScope::Custom { description } => description.clone(),
                }
            }

            match guard_event {
                GuardEvent::VerificationStarted {
                    scope,
                    level,
                    targets,
                    ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        scope = %scope_label(scope),
                        level = ?level,
                        packages = targets.packages,
                        files = ?targets.files,
                        "Guard verification started"
                    );
                }
                GuardEvent::VerificationCompleted {
                    scope,
                    discrepancies,
                    metrics,
                    ..
                } => {
                    if *discrepancies == 0 {
                        info!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            scope = %scope_label(scope),
                            coverage = metrics.coverage_percent,
                            cache_hit_rate = metrics.cache_hit_rate,
                            duration_ms = metrics.duration_ms,
                            "Guard verification completed",
                        );
                    } else {
                        warn!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            scope = %scope_label(scope),
                            discrepancies = *discrepancies,
                            coverage = metrics.coverage_percent,
                            cache_hit_rate = metrics.cache_hit_rate,
                            duration_ms = metrics.duration_ms,
                            "Guard verification completed with findings",
                        );
                    }
                }
                GuardEvent::VerificationFailed { scope, failure, .. } => {
                    if failure.retryable {
                        warn!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                        scope = %scope_label(scope),
                            retryable = failure.retryable,
                            code = ?failure.code,
                            message = %failure.message,
                            hint = ?failure.hint,
                            "Guard verification failed",
                        );
                    } else {
                        error!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            scope = %scope_label(scope),
                            retryable = failure.retryable,
                            code = ?failure.code,
                            message = %failure.message,
                            hint = ?failure.hint,
                            "Guard verification failed",
                        );
                    }
                }
                GuardEvent::HealingStarted { plan, .. } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        total = plan.total,
                        auto = plan.auto_heal,
                        confirmation = plan.confirmation_required,
                        manual = plan.manual_only,
                        "Guard healing started",
                    );
                }
                GuardEvent::HealingCompleted {
                    healed,
                    failed,
                    duration_ms,
                    ..
                } => {
                    if *failed == 0 {
                        info!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            healed = *healed,
                            failed = *failed,
                            duration_ms = *duration_ms,
                            "Guard healing completed",
                        );
                    } else {
                        warn!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            healed = *healed,
                            failed = *failed,
                            duration_ms = *duration_ms,
                            "Guard healing completed with failures",
                        );
                    }
                }
                GuardEvent::HealingFailed {
                    failure, healed, ..
                } => {
                    if failure.retryable {
                        warn!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            healed = *healed,
                            retryable = failure.retryable,
                            code = ?failure.code,
                            message = %failure.message,
                            hint = ?failure.hint,
                            "Guard healing failed",
                        );
                    } else {
                        error!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            healed = *healed,
                            retryable = failure.retryable,
                            code = ?failure.code,
                            message = %failure.message,
                            hint = ?failure.hint,
                            "Guard healing failed",
                        );
                    }
                }
                GuardEvent::DiscrepancyReported { discrepancy, .. } => {
                    let severity = match discrepancy.severity {
                        GuardSeverity::Critical => "critical",
                        GuardSeverity::High => "high",
                        GuardSeverity::Medium => "medium",
                        GuardSeverity::Low => "low",
                    };
                    warn!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        severity,
                        kind = %discrepancy.kind,
                        location = ?discrepancy.location,
                        package = ?discrepancy.package,
                        version = ?discrepancy.version,
                        auto_heal = discrepancy.auto_heal_available,
                        confirmation = discrepancy.requires_confirmation,
                        message = %discrepancy.message,
                        "Guard discrepancy reported",
                    );
                }
            }
        }

        AppEvent::Lifecycle(sps2_events::events::LifecycleEvent::Resolver {
            stage,
            context,
            failure,
        }) => {
            use sps2_events::events::LifecycleStage;
            match stage {
                LifecycleStage::Started => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        runtime_targets = ?context.runtime_targets,
                        build_targets = ?context.build_targets,
                        local_targets = ?context.local_targets,
                        "Dependency resolution started"
                    );
                }
                LifecycleStage::Completed => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        total_packages = ?context.total_packages,
                        downloaded_packages = ?context.downloaded_packages,
                        reused_packages = ?context.reused_packages,
                        duration_ms = ?context.duration_ms,
                        "Dependency resolution completed"
                    );
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        if failure_ctx.retryable {
                            warn!(
                                source = meta.source.as_str(),
                                event_id = %meta.event_id,
                                correlation = ?meta.correlation_id,
                                retryable = failure_ctx.retryable,
                                code = ?failure_ctx.code,
                                message = %failure_ctx.message,
                                hint = ?failure_ctx.hint,
                                conflicts = ?context.conflicting_packages,
                                "Dependency resolution failed"
                            );
                        } else {
                            error!(
                                source = meta.source.as_str(),
                                event_id = %meta.event_id,
                                correlation = ?meta.correlation_id,
                                retryable = failure_ctx.retryable,
                                code = ?failure_ctx.code,
                                message = %failure_ctx.message,
                                hint = ?failure_ctx.hint,
                                conflicts = ?context.conflicting_packages,
                                "Dependency resolution failed"
                            );
                        }
                    }
                }
            }
        }

        // Install domain events
        AppEvent::Lifecycle(sps2_events::events::LifecycleEvent::Install {
            stage,
            context,
            failure,
        }) => {
            use sps2_events::events::LifecycleStage;
            match stage {
                LifecycleStage::Started => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = ?context.package,
                        version = ?context.version,
                        "Package installation started"
                    );
                }
                LifecycleStage::Completed => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = ?context.package,
                        version = ?context.version,
                        files_installed = ?context.files_installed,
                        "Package installation completed"
                    );
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        error!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            package = ?context.package,
                            version = ?context.version,
                            retryable = failure_ctx.retryable,
                            code = ?failure_ctx.code,
                            message = %failure_ctx.message,
                            hint = ?failure_ctx.hint,
                            "Package installation failed"
                        );
                    }
                }
            }
        }

        // Uninstall domain events
        AppEvent::Lifecycle(sps2_events::events::LifecycleEvent::Uninstall {
            stage,
            context,
            failure,
        }) => {
            use sps2_events::events::LifecycleStage;
            match stage {
                LifecycleStage::Started => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = ?context.package,
                        version = ?context.version,
                        "Package uninstallation started"
                    );
                }
                LifecycleStage::Completed => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = ?context.package,
                        version = ?context.version,
                        files_removed = ?context.files_removed,
                        "Package uninstallation completed"
                    );
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        if failure_ctx.retryable {
                            warn!(
                                source = meta.source.as_str(),
                                event_id = %meta.event_id,
                                correlation = ?meta.correlation_id,
                                package = ?context.package,
                                version = ?context.version,
                                retryable = failure_ctx.retryable,
                                code = ?failure_ctx.code,
                                message = %failure_ctx.message,
                                hint = ?failure_ctx.hint,
                                "Package uninstallation failed"
                            );
                        } else {
                            error!(
                                source = meta.source.as_str(),
                                event_id = %meta.event_id,
                                correlation = ?meta.correlation_id,
                                package = ?context.package,
                                version = ?context.version,
                                retryable = failure_ctx.retryable,
                                code = ?failure_ctx.code,
                                message = %failure_ctx.message,
                                hint = ?failure_ctx.hint,
                                "Package uninstallation failed"
                            );
                        }
                    }
                }
            }
        }

        AppEvent::Qa(qa_event) => {
            use sps2_events::{events::QaCheckStatus, QaEvent};
            match qa_event {
                QaEvent::PipelineStarted { target, level } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %target.package,
                        version = %target.version,
                        level = ?level,
                        "QA pipeline started"
                    );
                }
                QaEvent::PipelineCompleted {
                    target,
                    total_checks,
                    passed,
                    failed,
                    duration_ms,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %target.package,
                        version = %target.version,
                        total_checks = total_checks,
                        passed = passed,
                        failed = failed,
                        duration_ms = duration_ms,
                        "QA pipeline completed"
                    );
                }
                QaEvent::PipelineFailed { target, failure } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %target.package,
                        version = %target.version,
                        code = ?failure.code,
                        retryable = failure.retryable,
                        hint = ?failure.hint,
                        message = %failure.message,
                        "QA pipeline failed"
                    );
                }
                QaEvent::CheckEvaluated { summary, .. } => {
                    let status_str = format!("{:?}", summary.status);
                    let severity = match summary.status {
                        QaCheckStatus::Passed => tracing::Level::INFO,
                        QaCheckStatus::Failed => tracing::Level::ERROR,
                        QaCheckStatus::Skipped => tracing::Level::DEBUG,
                    };
                    match severity {
                        tracing::Level::ERROR => error!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            check_name = %summary.name,
                            category = %summary.category,
                            status = %status_str,
                            findings = summary.findings.len(),
                            duration_ms = summary.duration_ms,
                            "QA check evaluated"
                        ),
                        tracing::Level::DEBUG => debug!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            check_name = %summary.name,
                            category = %summary.category,
                            status = %status_str,
                            findings = summary.findings.len(),
                            duration_ms = summary.duration_ms,
                            "QA check evaluated"
                        ),
                        _ => info!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            check_name = %summary.name,
                            category = %summary.category,
                            status = %status_str,
                            findings = summary.findings.len(),
                            duration_ms = summary.duration_ms,
                            "QA check evaluated"
                        ),
                    }
                }
            }
        }

        AppEvent::Package(package_event) => {
            use sps2_events::PackageEvent;
            match package_event {
                PackageEvent::OperationStarted { operation } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = ?operation,
                        "Package operation started"
                    );
                }
                PackageEvent::OperationCompleted { operation, outcome } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = ?operation,
                        outcome = ?outcome,
                        "Package operation completed"
                    );
                }
                PackageEvent::OperationFailed { operation, failure } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = ?operation,
                        code = ?failure.code,
                        retryable = failure.retryable,
                        hint = ?failure.hint,
                        message = %failure.message,
                        "Package operation failed"
                    );
                }
            }
        }

        // State domain events
        AppEvent::State(state_event) => {
            use sps2_events::StateEvent;
            match state_event {
                StateEvent::TransitionStarted { context } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = %context.operation,
                        source_state = ?context.source,
                        target_state = %context.target,
                        "State transition started"
                    );
                }
                StateEvent::TransitionCompleted { context, summary } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = %context.operation,
                        source_state = ?context.source,
                        target_state = %context.target,
                        duration_ms = summary.as_ref().and_then(|s| s.duration_ms),
                        "State transition completed"
                    );
                }
                StateEvent::TransitionFailed { context, failure } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = %context.operation,
                        source_state = ?context.source,
                        target_state = %context.target,
                        code = ?failure.code,
                        retryable = failure.retryable,
                        hint = ?failure.hint,
                        message = %failure.message,
                        "State transition failed"
                    );
                }
                StateEvent::RollbackStarted { context } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        from_state = %context.from,
                        to_state = %context.to,
                        "Rollback started"
                    );
                }
                StateEvent::RollbackCompleted { context, summary } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        from_state = %context.from,
                        to_state = %context.to,
                        duration_ms = summary.as_ref().and_then(|s| s.duration_ms),
                        "Rollback completed"
                    );
                }
                StateEvent::RollbackFailed { context, failure } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        from_state = %context.from,
                        to_state = %context.to,
                        code = ?failure.code,
                        retryable = failure.retryable,
                        hint = ?failure.hint,
                        message = %failure.message,
                        "Rollback failed"
                    );
                }
                StateEvent::CleanupStarted { summary } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        planned_states = summary.planned_states,
                        "Cleanup started"
                    );
                }
                StateEvent::CleanupCompleted { summary } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        planned_states = summary.planned_states,
                        removed_states = summary.removed_states,
                        space_freed_bytes = summary.space_freed_bytes,
                        duration_ms = summary.duration_ms,
                        "Cleanup completed"
                    );
                }
                StateEvent::CleanupFailed { summary, failure } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        planned_states = summary.planned_states,
                        removed_states = summary.removed_states,
                        space_freed_bytes = summary.space_freed_bytes,
                        code = ?failure.code,
                        retryable = failure.retryable,
                        hint = ?failure.hint,
                        message = %failure.message,
                        "Cleanup failed"
                    );
                }
            }
        }

        // Update/upgrade events
        AppEvent::Lifecycle(sps2_events::events::LifecycleEvent::Update {
            stage,
            context,
            failure,
        }) => {
            use sps2_events::events::LifecycleStage;
            match stage {
                LifecycleStage::Started => {
                    let total_targets = context.requested.as_ref().map(|v| v.len()).unwrap_or(0);
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = ?context.operation,
                        requested = ?context.requested,
                        total_targets,
                        "Update operation started"
                    );
                }
                LifecycleStage::Completed => {
                    let updated_len = context.updated.as_ref().map(|v| v.len()).unwrap_or(0);
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = ?context.operation,
                        updated = updated_len,
                        skipped = ?context.skipped,
                        duration_ms = ?context.duration.map(|d| d.as_millis()),
                        size_difference = ?context.size_difference,
                        "Update operation completed"
                    );
                }
                LifecycleStage::Failed => {
                    if let Some(failure_ctx) = failure {
                        let updated_len = context.updated.as_ref().map(|v| v.len()).unwrap_or(0);
                        let failed_len = context.failed.as_ref().map(|v| v.len()).unwrap_or(0);
                        if failure_ctx.retryable {
                            warn!(
                                source = meta.source.as_str(),
                                event_id = %meta.event_id,
                                correlation = ?meta.correlation_id,
                                operation = ?context.operation,
                                completed = updated_len,
                                failed = failed_len,
                                code = ?failure_ctx.code,
                                message = %failure_ctx.message,
                                hint = ?failure_ctx.hint,
                                "Update operation failed"
                            );
                        } else {
                            error!(
                                source = meta.source.as_str(),
                                event_id = %meta.event_id,
                                correlation = ?meta.correlation_id,
                                operation = ?context.operation,
                                completed = updated_len,
                                failed = failed_len,
                                code = ?failure_ctx.code,
                                message = %failure_ctx.message,
                                hint = ?failure_ctx.hint,
                                "Update operation failed"
                            );
                        }
                    }
                }
            }
        }

        // General domain events
        AppEvent::General(general_event) => {
            use sps2_events::GeneralEvent;
            match general_event {
                GeneralEvent::OperationStarted { operation } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = %operation,
                        "Operation started"
                    );
                }
                GeneralEvent::OperationCompleted { operation, success } => {
                    if *success {
                        info!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            operation = %operation,
                            success = success,
                            "Operation completed successfully"
                        );
                    } else {
                        warn!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            operation = %operation,
                            success = success,
                            "Operation completed with issues"
                        );
                    }
                }
                GeneralEvent::OperationFailed { operation, failure } => {
                    if failure.retryable {
                        warn!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            operation = %operation,
                            retryable = failure.retryable,
                            code = ?failure.code,
                            message = %failure.message,
                            hint = ?failure.hint,
                            "Operation failed"
                        );
                    } else {
                        error!(
                            source = meta.source.as_str(),
                            event_id = %meta.event_id,
                            correlation = ?meta.correlation_id,
                            operation = %operation,
                            retryable = failure.retryable,
                            code = ?failure.code,
                            message = %failure.message,
                            hint = ?failure.hint,
                            "Operation failed"
                        );
                    }
                }
                GeneralEvent::Warning { message, context } => {
                    warn!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        message = %message,
                        context = ?context,
                        "Warning"
                    );
                }
                GeneralEvent::Error { message, details } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        message = %message,
                        details = ?details,
                        "Error"
                    );
                }
                GeneralEvent::DebugLog { message, context } => {
                    debug!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        message = %message,
                        context = ?context,
                        "Debug log"
                    );
                }
                _ => {
                    // Fallback for other general events
                    match level {
                        tracing::Level::ERROR => {
                            error!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?general_event, "General event")
                        }
                        tracing::Level::WARN => {
                            warn!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?general_event, "General event")
                        }
                        tracing::Level::INFO => {
                            info!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?general_event, "General event")
                        }
                        tracing::Level::DEBUG => {
                            debug!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?general_event, "General event")
                        }
                        tracing::Level::TRACE => {
                            trace!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?general_event, "General event")
                        }
                    }
                }
            }
        }

        // Fallback for all other event domains
        _ => match level {
            tracing::Level::ERROR => {
                error!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?event, "Application event")
            }
            tracing::Level::WARN => {
                warn!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?event, "Application event")
            }
            tracing::Level::INFO => {
                info!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?event, "Application event")
            }
            tracing::Level::DEBUG => {
                debug!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?event, "Application event")
            }
            tracing::Level::TRACE => {
                trace!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?event, "Application event")
            }
        },
    }
}

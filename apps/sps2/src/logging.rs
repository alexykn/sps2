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
        AppEvent::Download(download_event) => {
            use sps2_events::DownloadEvent;
            match download_event {
                DownloadEvent::Started {
                    url, total_size, ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %url,
                        size = ?total_size,
                        "Download started"
                    );
                }
                DownloadEvent::Progress {
                    url,
                    bytes_downloaded,
                    total_bytes,
                    ..
                } => {
                    let progress_percent = if *total_bytes == 0 {
                        None
                    } else {
                        Some((*bytes_downloaded as f64 / *total_bytes as f64) * 100.0)
                    };
                    debug!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %url,
                        bytes_downloaded = bytes_downloaded,
                        total_bytes = ?total_bytes,
                        progress_percent = ?progress_percent,
                        "Download progress"
                    );
                }
                DownloadEvent::Completed {
                    url,
                    final_size,
                    total_time,
                    ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %url,
                        size = final_size,
                        duration_ms = total_time.as_millis(),
                        "Download completed"
                    );
                }
                DownloadEvent::Failed { url, error, .. } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %url,
                        error = %error,
                        "Download failed"
                    );
                }
                _ => {
                    // Fallback for other download events
                    match level {
                        tracing::Level::ERROR => {
                            error!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?download_event, "Download event")
                        }
                        tracing::Level::WARN => {
                            warn!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?download_event, "Download event")
                        }
                        tracing::Level::INFO => {
                            info!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?download_event, "Download event")
                        }
                        tracing::Level::DEBUG => {
                            debug!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?download_event, "Download event")
                        }
                        tracing::Level::TRACE => {
                            trace!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?download_event, "Download event")
                        }
                    }
                }
            }
        }

        // Install domain events
        AppEvent::Install(install_event) => {
            use sps2_events::InstallEvent;
            match install_event {
                InstallEvent::Started {
                    package,
                    version,
                    install_path,
                    ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        install_path = %install_path.display(),
                        "Package installation started"
                    );
                }
                InstallEvent::Completed {
                    package,
                    version,
                    installed_files,
                    duration,
                    disk_usage,
                    ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        installed_files = installed_files,
                        duration_ms = duration.as_millis(),
                        disk_usage_bytes = disk_usage,
                        "Package installation completed"
                    );
                }
                InstallEvent::Failed {
                    package,
                    version,
                    phase,
                    error,
                    ..
                } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        phase = ?phase,
                        error = %error,
                        "Package installation failed"
                    );
                }
                InstallEvent::StagingStarted {
                    package,
                    version,
                    source_path,
                    staging_path,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        source_path = %source_path.display(),
                        staging_path = %staging_path.display(),
                        "Package staging started"
                    );
                }
                InstallEvent::StagingCompleted {
                    package,
                    version,
                    files_staged,
                    staging_size,
                    ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        files_staged = files_staged,
                        staging_size_bytes = staging_size,
                        "Package staging completed"
                    );
                }
                InstallEvent::ValidationStarted {
                    package,
                    version,
                    validation_checks,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        validation_checks = ?validation_checks,
                        "Package validation started"
                    );
                }
                InstallEvent::ValidationCompleted {
                    package,
                    version,
                    checks_passed,
                    warnings,
                    issues_found,
                } => {
                    if *issues_found > 0 {
                        warn!(
                            source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                            package = %package,
                            version = %version,
                            checks_passed = checks_passed,
                            warnings = warnings,
                            issues_found = issues_found,
                            "Package validation completed with issues"
                        );
                    } else {
                        info!(
                            source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                            package = %package,
                            version = %version,
                            checks_passed = checks_passed,
                            warnings = warnings,
                            "Package validation completed successfully"
                        );
                    }
                }
                _ => {
                    // Fallback for other install events
                    match level {
                        tracing::Level::ERROR => {
                            error!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?install_event, "Install event")
                        }
                        tracing::Level::WARN => {
                            warn!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?install_event, "Install event")
                        }
                        tracing::Level::INFO => {
                            info!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?install_event, "Install event")
                        }
                        tracing::Level::DEBUG => {
                            debug!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?install_event, "Install event")
                        }
                        tracing::Level::TRACE => {
                            trace!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?install_event, "Install event")
                        }
                    }
                }
            }
        }

        // State domain events
        AppEvent::State(state_event) => {
            use sps2_events::StateEvent;
            match state_event {
                StateEvent::Created {
                    state_id,
                    operation,
                    ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        state_id = %state_id,
                        operation = %operation,
                        "State created"
                    );
                }
                StateEvent::TransitionCompleted {
                    from,
                    to,
                    operation,
                    ..
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        from_state = %from,
                        to_state = %to,
                        operation = %operation,
                        "State transition completed"
                    );
                }
                StateEvent::TwoPhaseCommitStarting {
                    state_id,
                    parent_state_id,
                    operation,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        state_id = %state_id,
                        parent_state_id = %parent_state_id,
                        operation = %operation,
                        "Two-phase commit started"
                    );
                }
                StateEvent::TwoPhaseCommitCompleted {
                    state_id,
                    parent_state_id,
                    operation,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        state_id = %state_id,
                        parent_state_id = %parent_state_id,
                        operation = %operation,
                        "Two-phase commit completed"
                    );
                }
                _ => {
                    // Fallback for other state events
                    match level {
                        tracing::Level::ERROR => {
                            error!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?state_event, "State event")
                        }
                        tracing::Level::WARN => {
                            warn!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?state_event, "State event")
                        }
                        tracing::Level::INFO => {
                            info!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?state_event, "State event")
                        }
                        tracing::Level::DEBUG => {
                            debug!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?state_event, "State event")
                        }
                        tracing::Level::TRACE => {
                            trace!(source = meta.source.as_str(), event_id = %meta.event_id, correlation = ?meta.correlation_id, event = ?state_event, "State event")
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

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
                    url,
                    package,
                    total_bytes,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %url,
                        package = ?package,
                        total_bytes = ?total_bytes,
                        "Download started"
                    );
                }
                DownloadEvent::Completed {
                    url,
                    package,
                    bytes_downloaded,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %url,
                        package = ?package,
                        bytes_downloaded = bytes_downloaded,
                        "Download completed"
                    );
                }
                DownloadEvent::Failed {
                    url,
                    package,
                    retryable,
                } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        url = %url,
                        package = ?package,
                        retryable = retryable,
                        "Download failed"
                    );
                }
            }
        }

        // Install domain events
        AppEvent::Install(install_event) => {
            use sps2_events::InstallEvent;
            match install_event {
                InstallEvent::Started { package, version } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        "Package installation started"
                    );
                }
                InstallEvent::Completed {
                    package,
                    version,
                    files_installed,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        files_installed = files_installed,
                        "Package installation completed"
                    );
                }
                InstallEvent::Failed {
                    package,
                    version,
                    retryable,
                } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        retryable = retryable,
                        "Package installation failed"
                    );
                }
            }
        }

        // Uninstall domain events
        AppEvent::Uninstall(uninstall_event) => {
            use sps2_events::UninstallEvent;
            match uninstall_event {
                UninstallEvent::Started { package, version } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        "Package uninstallation started"
                    );
                }
                UninstallEvent::Completed {
                    package,
                    version,
                    files_removed,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        files_removed = files_removed,
                        "Package uninstallation completed"
                    );
                }
                UninstallEvent::Failed {
                    package,
                    version,
                    retryable,
                } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        package = %package,
                        version = %version,
                        retryable = retryable,
                        "Package uninstallation failed"
                    );
                }
            }
        }

        // State domain events
        AppEvent::State(state_event) => {
            use sps2_events::StateEvent;
            match state_event {
                StateEvent::TransitionStarted {
                    operation,
                    source,
                    target,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = %operation,
                        source_state = ?source,
                        target_state = %target,
                        "State transition started"
                    );
                }
                StateEvent::TransitionCompleted {
                    operation,
                    source,
                    target,
                    duration,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = %operation,
                        source_state = ?source,
                        target_state = %target,
                        duration_ms = duration.map(|d| d.as_millis()),
                        "State transition completed"
                    );
                }
                StateEvent::TransitionFailed {
                    operation,
                    source,
                    target,
                    retryable,
                } => {
                    error!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        operation = %operation,
                        source_state = ?source,
                        target_state = ?target,
                        retryable = retryable,
                        "State transition failed"
                    );
                }
                StateEvent::RollbackStarted { from, to } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        from_state = %from,
                        to_state = %to,
                        "Rollback started"
                    );
                }
                StateEvent::RollbackCompleted { from, to, duration } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        from_state = %from,
                        to_state = %to,
                        duration_ms = duration.map(|d| d.as_millis()),
                        "Rollback completed"
                    );
                }
                StateEvent::CleanupStarted { planned_states } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        planned_states = planned_states,
                        "Cleanup started"
                    );
                }
                StateEvent::CleanupCompleted {
                    removed_states,
                    space_freed,
                } => {
                    info!(
                        source = meta.source.as_str(),
                        event_id = %meta.event_id,
                        correlation = ?meta.correlation_id,
                        removed_states = removed_states,
                        space_freed_bytes = space_freed,
                        "Cleanup completed"
                    );
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

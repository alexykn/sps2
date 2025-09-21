use serde::{Deserialize, Serialize};

/// Resolver domain events for dependency resolution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResolverEvent {
    /// High-level resolution flow started
    Started {
        runtime_targets: usize,
        build_targets: usize,
        local_targets: usize,
    },

    /// Resolution completed successfully
    Completed {
        total_packages: usize,
        downloaded_packages: usize,
        reused_packages: usize,
        duration_ms: u64,
    },

    /// Resolution failed
    Failed {
        failure: super::FailureContext,
        conflicting_packages: Vec<String>,
    },
}

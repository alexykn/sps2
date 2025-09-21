use serde::{Deserialize, Serialize};

/// Resolver domain events for dependency resolution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResolverEvent {
    /// High-level resolution flow started
    ResolutionStarted {
        runtime_deps: usize,
        build_deps: usize,
        local_files: usize,
    },

    /// Resolution completed successfully
    ResolutionCompleted {
        total_packages: usize,
        duration_ms: u64,
    },

    /// Resolution failed
    ResolutionFailed { reason: ResolutionFailureReason },
}

/// Reasons for resolution failure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionFailureReason {
    /// A dependency conflict was detected
    Conflict(DependencyConflict),
    /// The resolution process timed out
    Timeout,
}

/// Details of a dependency conflict
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyConflict {
    pub conflicting_packages: Vec<(String, String)>, // (package, version)
    pub message: String,
    pub conflict_type: DependencyConflictType,
}

/// Types of dependency conflicts for categorization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyConflictType {
    /// Version constraints cannot be satisfied simultaneously
    VersionIncompatibility,
    /// Packages that cannot be installed together
    MutualExclusion,
    /// Circular dependency chain detected
    CircularDependency,
    /// Required dependency is not available
    MissingDependency,
    /// User constraint violated by solution
    ConstraintViolation,
    /// Platform or architecture incompatibility
    PlatformIncompatibility,
}

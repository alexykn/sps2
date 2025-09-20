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
        timeout_seconds: u64,
    },

    /// Resolution completed successfully
    ResolutionCompleted {
        total_packages: usize,
        execution_batches: usize,
        duration_ms: u64,
        packages_resolved: Vec<String>,
    },

    /// Dependency conflict detected
    DependencyConflictDetected {
        conflicting_packages: Vec<(String, String)>, // (package, version)
        message: String,
        conflict_type: DependencyConflictType,
        suggestion_count: usize,
    },

    /// Conflict resolution suggestions generated
    DependencyConflictSuggestions {
        suggestions: Vec<String>,
        automated_resolution_possible: bool,
        confidence_score: f64,
    },
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

impl ResolverEvent {
    /// Create a conflict detected event with basic info
    #[must_use]
    pub fn conflict_detected(packages: Vec<(String, String)>, message: String) -> Self {
        Self::DependencyConflictDetected {
            conflicting_packages: packages,
            message,
            conflict_type: DependencyConflictType::VersionIncompatibility,
            suggestion_count: 0,
        }
    }

    /// Create a conflict suggestions event
    #[must_use]
    pub fn conflict_suggestions(suggestions: Vec<String>) -> Self {
        Self::DependencyConflictSuggestions {
            suggestions,
            automated_resolution_possible: false,
            confidence_score: 0.5,
        }
    }
}

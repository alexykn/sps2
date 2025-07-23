use serde::{Deserialize, Serialize};

/// Resolver domain events for dependency resolution and SAT solving
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

    /// Resolution failed after all attempts
    ResolutionFailed {
        reason: String,
        attempts: u32,
        partial_solution: Option<Vec<String>>,
    },

    /// Resolution timed out
    ResolutionTimedOut {
        timeout_seconds: u64,
        packages_processed: usize,
        current_phase: String,
    },

    /// Installed package checking phase started
    InstalledPackageCheckStarted { dependency_count: usize },

    /// Installed package satisfies dependency
    InstalledPackageSatisfied {
        package: String,
        version: String,
        spec: String,
        installation_path: Option<String>,
    },

    /// Installed package check failed
    InstalledPackageCheckFailed {
        package: String,
        spec: String,
        reason: String,
    },

    /// SAT solving phase started
    SatSolvingStarted {
        variables: usize,
        clauses: usize,
        required_packages: usize,
        optional_packages: usize,
    },

    /// SAT solving progress update
    SatSolvingProgress {
        decisions: u64,
        propagations: u64,
        conflicts: u64,
        learned_clauses: u64,
        restarts: u64,
        current_level: u32,
    },

    /// SAT conflict detected during solving
    SatConflictDetected {
        conflict_level: u32,
        learned_clause_size: usize,
        backtrack_level: u32,
        conflicting_variables: Vec<String>,
    },

    /// SAT restart triggered by heuristic
    SatRestartTriggered {
        restart_count: u64,
        conflicts_since_last: u64,
        reason: String, // "luby_sequence", "geometric", "threshold"
    },

    /// SAT variable assignment made
    SatVariableAssigned {
        variable: String,
        value: bool,
        decision_level: u32,
        reason: String, // "decision", "unit_propagation", "conflict_analysis"
    },

    /// SAT solving completed
    SatSolvingCompleted {
        solution_found: bool,
        final_stats: SolverStats,
        duration_ms: u64,
        learned_clauses_retained: usize,
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

    /// Conflict analysis started
    ConflictAnalysisStarted {
        conflict_clause_size: usize,
        current_level: u32,
    },

    /// Conflict analysis completed
    ConflictAnalysisCompleted {
        learned_clause_size: usize,
        backtrack_level: u32,
        analysis_duration_ms: u64,
    },

    /// Dependency graph construction started
    DependencyGraphBuilding {
        expected_nodes: usize,
        expected_edges: usize,
    },

    /// Dependency graph construction completed
    DependencyGraphBuilt {
        node_count: usize,
        edge_count: usize,
        max_depth: usize,
        strongly_connected_components: usize,
    },

    /// Topological sort started
    TopologicalSortStarted {
        node_count: usize,
        edge_count: usize,
    },

    /// Topological sort completed
    TopologicalSortCompleted {
        execution_batches: usize,
        parallel_capacity: usize,
        critical_path_length: usize,
    },

    /// Circular dependency detected
    CycleDetected {
        cycle_packages: Vec<String>,
        cycle_length: usize,
        break_suggestions: Vec<String>,
    },

    /// Execution plan generated
    ExecutionPlanGenerated {
        batch_count: usize,
        parallel_packages: usize,
        sequential_packages: usize,
        estimated_duration_minutes: Option<f64>,
    },

    /// Version constraint processing started
    VersionConstraintProcessing {
        package: String,
        constraint_spec: String,
        available_versions: usize,
    },

    /// Version constraint processing completed
    VersionConstraintResolved {
        package: String,
        selected_version: String,
        matching_versions: usize,
        selection_reason: String, // "latest", "constraint", "dependency"
    },

    /// Transitive dependency discovered
    TransitiveDependencyDiscovered {
        parent_package: String,
        dependency_package: String,
        dependency_type: String, // "runtime", "build", "optional"
        depth: u32,
        version_constraint: String,
    },

    /// Dependency chain analysis
    DependencyChainAnalyzed {
        root_package: String,
        chain_length: usize,
        total_dependencies: usize,
        unique_packages: usize,
        potential_conflicts: usize,
    },

    /// Local package processing started
    LocalPackageProcessingStarted {
        file_path: String,
        expected_format: String,
    },

    /// Local manifest extracted successfully
    LocalManifestExtracted {
        package_name: String,
        version: String,
        dependencies: usize,
        file_path: String,
        manifest_size: usize,
    },

    /// Local package processing failed
    LocalPackageProcessingFailed {
        file_path: String,
        error: String,
        stage: String, // "extraction", "parsing", "validation"
    },

    /// Constraint propagation started
    ConstraintPropagationStarted {
        active_constraints: usize,
        unassigned_variables: usize,
    },

    /// Unit clause detected during propagation
    UnitClauseDetected {
        variable: String,
        forced_value: bool,
        clause_id: usize,
    },

    /// Constraint propagation completed
    ConstraintPropagationCompleted {
        propagated_assignments: usize,
        new_unit_clauses: usize,
        conflicts_detected: usize,
    },

    /// Solution validation started
    SolutionValidationStarted {
        packages_to_validate: usize,
        constraints_to_check: usize,
    },

    /// Solution validation completed
    SolutionValidationCompleted {
        validation_passed: bool,
        constraint_violations: usize,
        warnings: usize,
    },
}

/// SAT solver statistics for detailed diagnostics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverStats {
    pub decisions: u64,
    pub propagations: u64,
    pub conflicts: u64,
    pub learned_clauses: u64,
    pub restarts: u64,
    pub deleted_clauses: u64,
    pub max_decision_level: u32,
    pub total_literals: usize,
    pub unit_clauses: usize,
    pub binary_clauses: usize,
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

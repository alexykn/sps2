//! SAT solver for dependency resolution
//!
//! This module implements a DPLL-based SAT solver optimized for package
//! dependency resolution. It supports:
//! - Version constraint clauses
//! - Conflict-driven clause learning
//! - Two-watched literal optimization
//! - VSIDS variable ordering heuristic

use semver::Version;
use sps2_errors::{Error, PackageError};
use sps2_events::{Event, EventSender};
use std::collections::{HashMap, HashSet};
use std::fmt;

mod clause;
mod conflict_analysis;
mod solver;
mod types;
mod variable_map;

pub use clause::{Clause, ClauseRef};
pub use conflict_analysis::ConflictAnalysis;
pub use solver::SatSolver;
pub use types::{Assignment, Literal, Variable};
pub use variable_map::VariableMap;

/// Package identifier with version for SAT solving
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageVersion {
    pub name: String,
    pub version: Version,
}

impl PackageVersion {
    /// Create new package version
    #[must_use]
    pub fn new(name: String, version: Version) -> Self {
        Self { name, version }
    }
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// SAT problem representing package dependency constraints
#[derive(Debug, Clone)]
pub struct DependencyProblem {
    /// Map from package versions to SAT variables
    pub variables: VariableMap,
    /// Clauses representing constraints
    pub clauses: Vec<Clause>,
    /// Required packages (at least one version must be selected)
    pub required_packages: HashSet<String>,
}

impl DependencyProblem {
    /// Create new dependency problem
    #[must_use]
    pub fn new() -> Self {
        Self {
            variables: VariableMap::new(),
            clauses: Vec::new(),
            required_packages: HashSet::new(),
        }
    }

    /// Add a package version to the problem
    pub fn add_package_version(&mut self, package: PackageVersion) -> Variable {
        self.variables.add_package_version(package)
    }

    /// Add a clause to the problem
    pub fn add_clause(&mut self, clause: Clause) {
        self.clauses.push(clause);
    }

    /// Mark a package as required
    pub fn require_package(&mut self, name: String) {
        self.required_packages.insert(name);
    }

    /// Get all versions of a package
    #[must_use]
    pub fn get_package_versions(&self, name: &str) -> Vec<&PackageVersion> {
        self.variables.get_package_versions(name)
    }

    /// Add constraint: at most one version of a package can be selected
    ///
    /// # Panics
    ///
    /// Panics if variables for package versions are not found. This should not
    /// happen in normal usage as versions are added before constraints.
    pub fn add_at_most_one_constraint(&mut self, package_name: &str) {
        // Clone versions to avoid borrow issues
        let versions: Vec<PackageVersion> = self
            .variables
            .get_package_versions(package_name)
            .into_iter()
            .cloned()
            .collect();

        // For each pair of versions, add clause: ¬v1 ∨ ¬v2
        for i in 0..versions.len() {
            for j in (i + 1)..versions.len() {
                let v1 = self.variables.get_variable(&versions[i]).unwrap();
                let v2 = self.variables.get_variable(&versions[j]).unwrap();

                let clause = Clause::new(vec![Literal::negative(v1), Literal::negative(v2)]);

                self.add_clause(clause);
            }
        }
    }

    /// Add constraint: at least one version of a required package must be selected
    pub fn add_at_least_one_constraint(&mut self, package_name: &str) {
        let versions = self.variables.get_package_versions(package_name);

        if !versions.is_empty() {
            let literals: Vec<Literal> = versions
                .iter()
                .filter_map(|pv| self.variables.get_variable(pv))
                .map(Literal::positive)
                .collect();

            if !literals.is_empty() {
                self.add_clause(Clause::new(literals));
            }
        }
    }
}

impl Default for DependencyProblem {
    fn default() -> Self {
        Self::new()
    }
}

/// Solution to a dependency problem
#[derive(Debug, Clone)]
pub struct DependencySolution {
    /// Selected package versions
    pub selected: HashMap<String, Version>,
    /// Assignment that satisfies all constraints
    pub assignment: Assignment,
}

impl DependencySolution {
    /// Create new solution
    #[must_use]
    pub fn new(selected: HashMap<String, Version>, assignment: Assignment) -> Self {
        Self {
            selected,
            assignment,
        }
    }

    /// Check if a package version is selected
    #[must_use]
    pub fn is_selected(&self, name: &str, version: &Version) -> bool {
        self.selected.get(name) == Some(version)
    }
}

/// Conflict explanation for unsatisfiable problems
#[derive(Debug, Clone)]
pub struct ConflictExplanation {
    /// Conflicting packages
    pub conflicting_packages: Vec<(String, String)>,
    /// Human-readable explanation
    pub message: String,
    /// Suggested resolutions
    pub suggestions: Vec<String>,
}

impl ConflictExplanation {
    /// Create new conflict explanation
    #[must_use]
    pub fn new(
        conflicting_packages: Vec<(String, String)>,
        message: String,
        suggestions: Vec<String>,
    ) -> Self {
        Self {
            conflicting_packages,
            message,
            suggestions,
        }
    }
}

/// Convert a dependency problem to CNF and solve using SAT
///
/// # Errors
///
/// Returns an error if:
/// - The SAT problem is unsatisfiable (conflicting constraints)
/// - The solver encounters an internal error
/// - Package version mapping fails
pub async fn solve_dependencies(
    problem: DependencyProblem,
    event_sender: Option<&EventSender>,
) -> Result<DependencySolution, Error> {
    // Create SAT solver with version preference
    let mut solver = SatSolver::with_variable_map(&problem.variables);

    // Add all clauses to the solver
    for clause in &problem.clauses {
        solver.add_clause(clause.clone());
    }

    // Add at-most-one constraints for each package
    let all_packages: HashSet<String> =
        problem.variables.all_packages().map(String::from).collect();

    for package in &all_packages {
        let versions = problem.variables.get_package_versions(package);

        // At most one version can be selected
        for i in 0..versions.len() {
            for j in (i + 1)..versions.len() {
                if let (Some(v1), Some(v2)) = (
                    problem.variables.get_variable(versions[i]),
                    problem.variables.get_variable(versions[j]),
                ) {
                    solver.add_clause(Clause::new(vec![
                        Literal::negative(v1),
                        Literal::negative(v2),
                    ]));
                }
            }
        }
    }

    // Add at-least-one constraints for required packages
    for package in &problem.required_packages {
        let versions = problem.variables.get_package_versions(package);
        let literals: Vec<Literal> = versions
            .iter()
            .filter_map(|pv| problem.variables.get_variable(pv))
            .map(Literal::positive)
            .collect();

        if !literals.is_empty() {
            solver.add_clause(Clause::new(literals));
        }
    }

    // Solve the SAT problem
    if let Ok(assignment) = solver.solve() {
        // Extract selected packages from assignment
        let mut selected = HashMap::new();

        for package_name in all_packages {
            let versions = problem.variables.get_package_versions(&package_name);

            for package_version in versions {
                if let Some(var) = problem.variables.get_variable(package_version) {
                    if assignment.is_true(var) {
                        selected.insert(
                            package_version.name.clone(),
                            package_version.version.clone(),
                        );
                        break;
                    }
                }
            }
        }

        Ok(DependencySolution::new(selected, assignment))
    } else {
        // Extract conflict information
        let conflict = solver.analyze_conflict(&problem);

        // Emit conflict events if sender is available
        if let Some(sender) = event_sender {
            // Emit detailed conflict information
            if !conflict.conflicting_packages.is_empty() {
                let _ = sender.send(Event::DependencyConflictDetected {
                    conflicting_packages: conflict.conflicting_packages.clone(),
                    message: conflict.message.clone(),
                });
            }

            // Emit suggestions for resolution
            if !conflict.suggestions.is_empty() {
                let _ = sender.send(Event::DependencyConflictSuggestions {
                    suggestions: conflict.suggestions.clone(),
                });
            }
        }

        Err(PackageError::DependencyConflict {
            message: conflict.message,
        }
        .into())
    }
}

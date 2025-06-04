//! DPLL-based SAT solver with CDCL optimizations

use super::conflict_analysis::VariableActivity;
use super::types::TruthValue;
use super::{
    Assignment, Clause, ClauseRef, ConflictAnalysis, ConflictExplanation, DependencyProblem,
    Literal, Variable,
};
use crate::sat::clause::clause_ref;
use sps2_errors::{Error, PackageError};
use std::collections::{HashMap, HashSet, VecDeque};

/// SAT solver using DPLL with conflict-driven clause learning
#[derive(Debug)]
pub struct SatSolver {
    /// Original clauses
    clauses: Vec<ClauseRef>,
    /// Learned clauses from conflicts
    learned_clauses: Vec<ClauseRef>,
    /// Current assignment
    assignment: Assignment,
    /// Propagation queue
    propagation_queue: VecDeque<Literal>,
    /// Watch lists for two-watched literal scheme
    /// Maps each literal to clauses watching it
    watch_lists: HashMap<Literal, Vec<ClauseRef>>,
    /// Conflict analysis
    conflict_analysis: ConflictAnalysis,
    /// Variable activity for VSIDS heuristic
    variable_activity: VariableActivity,
    /// All variables in the problem
    variables: HashSet<Variable>,
    /// Decision stack for backtracking
    decisions: Vec<(Variable, bool)>,
    /// Statistics
    stats: SolverStats,
}

/// Solver statistics
#[derive(Debug, Default)]
struct SolverStats {
    decisions: u64,
    propagations: u64,
    conflicts: u64,
    learned_clauses: u64,
    restarts: u64,
}

impl SatSolver {
    /// Create new SAT solver
    #[must_use]
    pub fn new() -> Self {
        Self {
            clauses: Vec::new(),
            learned_clauses: Vec::new(),
            assignment: Assignment::new(),
            propagation_queue: VecDeque::new(),
            watch_lists: HashMap::new(),
            conflict_analysis: ConflictAnalysis::new(),
            variable_activity: VariableActivity::new(0.95),
            variables: HashSet::new(),
            decisions: Vec::new(),
            stats: SolverStats::default(),
        }
    }

    /// Create new SAT solver with variable map for version preference
    #[must_use]
    pub fn with_variable_map(variable_map: &super::VariableMap) -> Self {
        let mut solver = Self::new();
        solver.set_version_preference(variable_map);
        solver
    }

    /// Set version preference based on variable map
    pub fn set_version_preference(&mut self, variable_map: &super::VariableMap) {
        // Boost activity for higher versions to prefer them in decisions
        for package_name in variable_map.all_packages() {
            let mut versions: Vec<_> = variable_map
                .get_package_versions(package_name)
                .into_iter()
                .filter_map(|pv| variable_map.get_variable(pv).map(|var| (pv, var)))
                .collect();

            // Sort by version descending
            versions.sort_by(|(pv1, _), (pv2, _)| pv2.version.cmp(&pv1.version));

            // Give higher activity to higher versions
            for (i, (_, var)) in versions.iter().enumerate() {
                // Boost activity multiple times to give preference to higher versions
                let boost_count = (versions.len() - i) * 10;
                for _ in 0..boost_count {
                    self.variable_activity.bump_variable(*var);
                }
            }
        }
    }

    /// Add a clause to the solver
    pub fn add_clause(&mut self, clause: Clause) {
        // Simplify clause
        let Some(simplified) = clause.simplify() else {
            return; // Tautology, skip
        };

        if simplified.is_empty() {
            // Empty clause means unsatisfiable
            // We'll handle this in solve()
        }

        // Extract variables
        for &lit in simplified.literals() {
            self.variables.insert(lit.variable());
        }

        let clause_ref = clause_ref(simplified);

        // Initialize watched literals
        if clause_ref.len() >= 2 {
            // Watch first two literals - we watch for when they become false
            let lit1 = clause_ref.literals()[0];
            let lit2 = clause_ref.literals()[1];

            // Add clause to watch lists for the NEGATION of these literals
            // When lit1 becomes false (i.e., ¬lit1 becomes true), we need to update watches
            self.watch_lists
                .entry(lit1.negate())
                .or_default()
                .push(clause_ref.clone());
            self.watch_lists
                .entry(lit2.negate())
                .or_default()
                .push(clause_ref.clone());
        } else if clause_ref.len() == 1 {
            // Unit clause - add to propagation queue
            self.propagation_queue.push_back(clause_ref.literals()[0]);
        }

        self.clauses.push(clause_ref);
    }

    /// Solve the SAT problem
    pub fn solve(&mut self) -> Result<Assignment, Error> {
        // Check for empty clauses (immediate UNSAT)
        if self.clauses.iter().any(|c| c.is_empty()) {
            return Err(PackageError::DependencyConflict {
                message: "Unsatisfiable constraints detected".to_string(),
            }
            .into());
        }

        // Main DPLL loop
        loop {
            // Unit propagation
            match self.propagate() {
                PropagationResult::Conflict(conflict_clause) => {
                    self.stats.conflicts += 1;

                    // Analyze conflict
                    self.conflict_analysis.set_conflict(conflict_clause);

                    if let Some((learned_clause, backtrack_level)) =
                        self.conflict_analysis.analyze_conflict(&self.assignment)
                    {
                        // Learn clause
                        self.learn_clause(learned_clause);

                        // Backtrack
                        self.backtrack_to(backtrack_level);
                    } else {
                        // Conflict at level 0 - UNSAT
                        return Err(PackageError::DependencyConflict {
                            message: "No valid package selection exists".to_string(),
                        }
                        .into());
                    }
                }
                PropagationResult::Ok => {
                    // Check if all variables are assigned
                    if self.all_variables_assigned() {
                        return Ok(self.assignment.clone());
                    }

                    // Make a decision
                    if let Some((var, value)) = self.decide() {
                        self.stats.decisions += 1;
                        self.decisions.push((var, value));
                        self.assignment
                            .assign(var, value, self.assignment.current_level() + 1);

                        let lit = if value {
                            Literal::positive(var)
                        } else {
                            Literal::negative(var)
                        };
                        self.propagation_queue.push_back(lit);
                    } else {
                        // No unassigned variables but not all assigned? Shouldn't happen
                        return Ok(self.assignment.clone());
                    }
                }
            }

            // Restart heuristic (every 100 conflicts)
            if self.stats.conflicts > 0 && self.stats.conflicts % 100 == 0 {
                self.restart();
                self.stats.restarts += 1;
            }
        }
    }

    /// Unit propagation with two-watched literals
    fn propagate(&mut self) -> PropagationResult {
        while let Some(lit) = self.propagation_queue.pop_front() {
            self.stats.propagations += 1;

            // First, check if this literal is already assigned
            let current_value = self.assignment.eval_literal(lit);
            if current_value.is_false() {
                // Conflict: trying to assign a literal that's already false
                // Find the unit clause that contains this literal for conflict analysis
                for clause in &self.clauses {
                    if clause.is_unit() && clause.literals()[0] == lit {
                        return PropagationResult::Conflict(clause.clone());
                    }
                }
                // If we can't find the clause, create a dummy one
                let dummy_clause = clause_ref(Clause::unit(lit));
                return PropagationResult::Conflict(dummy_clause);
            } else if current_value == TruthValue::Unassigned {
                // Assign the literal
                let var = lit.variable();
                let assign_value = lit.is_positive();
                self.assignment
                    .assign(var, assign_value, self.assignment.current_level());
            }

            // When literal L becomes true, we check clauses watching L
            // because we watch for when literals become false
            if let Some(watching) = self.watch_lists.get(&lit).cloned() {
                for clause_ref in watching {
                    match self.update_watches(&clause_ref, lit) {
                        WatchResult::Conflict => {
                            self.propagation_queue.clear();
                            return PropagationResult::Conflict(clause_ref);
                        }
                        WatchResult::Unit(unit_lit) => {
                            // Check if already assigned
                            let value = self.assignment.eval_literal(unit_lit);
                            if value.is_false() {
                                // Conflict
                                self.propagation_queue.clear();
                                return PropagationResult::Conflict(clause_ref);
                            } else if value == TruthValue::Unassigned {
                                // Propagate
                                let var = unit_lit.variable();
                                let assign_value = unit_lit.is_positive();
                                self.assignment.assign(
                                    var,
                                    assign_value,
                                    self.assignment.current_level(),
                                );
                                self.propagation_queue.push_back(unit_lit);

                                // Record implication for conflict analysis
                                self.conflict_analysis.record_implication(
                                    var,
                                    clause_ref.clone(),
                                    self.assignment.current_level(),
                                );
                            }
                        }
                        WatchResult::Ok => {}
                    }
                }
            }
        }

        PropagationResult::Ok
    }

    /// Update watches for a clause when a literal becomes false
    fn update_watches(&mut self, clause: &ClauseRef, assigned_lit: Literal) -> WatchResult {
        // The assigned_lit just became true, so its negation became false
        let lit_that_became_false = assigned_lit.negate();

        // First, check if clause is already satisfied
        for &lit in clause.literals() {
            if self.assignment.eval_literal(lit).is_true() {
                return WatchResult::Ok;
            }
        }

        // Find the two currently watched literals (should be the first two that aren't false)
        let mut watched_indices = Vec::new();
        let mut other_unassigned_indices = Vec::new();
        let mut false_count = 0;
        let mut unassigned_count = 0;

        // Check all literals in the clause
        for (i, &lit) in clause.literals().iter().enumerate() {
            let value = self.assignment.eval_literal(lit);

            if value == TruthValue::Unassigned {
                unassigned_count += 1;
                // Check if this literal is currently being watched
                if self.watch_lists.get(&lit.negate()).is_some_and(|list| {
                    list.iter()
                        .any(|c| std::ptr::eq(c.as_ref(), clause.as_ref()))
                }) {
                    watched_indices.push(i);
                } else {
                    other_unassigned_indices.push(i);
                }
            } else if value.is_false() {
                false_count += 1;
                // Check if this false literal is currently being watched
                if lit == lit_that_became_false
                    && self.watch_lists.get(&assigned_lit).is_some_and(|list| {
                        list.iter()
                            .any(|c| std::ptr::eq(c.as_ref(), clause.as_ref()))
                    })
                {
                    watched_indices.push(i);
                }
            }
        }

        // If all literals are false, it's a conflict
        if false_count == clause.len() {
            return WatchResult::Conflict;
        }

        // If only one literal is unassigned and all others are false, it's a unit clause
        if unassigned_count == 1 && false_count == clause.len() - 1 {
            for &lit in clause.literals() {
                if self.assignment.eval_literal(lit) == TruthValue::Unassigned {
                    return WatchResult::Unit(lit);
                }
            }
        }

        // If we have at least 2 unassigned literals, maintain two watches
        if unassigned_count >= 2 {
            // We need to ensure exactly 2 literals are watched
            // Remove the watch for the literal that just became false
            if let Some(list) = self.watch_lists.get_mut(&assigned_lit) {
                list.retain(|c| !std::ptr::eq(c.as_ref(), clause.as_ref()));
            }

            // If we now have less than 2 watches, add a new one
            if watched_indices.len() < 2 && !other_unassigned_indices.is_empty() {
                let new_watched_idx = other_unassigned_indices[0];
                let new_watched_lit = clause.literals()[new_watched_idx];
                self.watch_lists
                    .entry(new_watched_lit.negate())
                    .or_default()
                    .push(clause.clone());
            }
        }

        WatchResult::Ok
    }

    /// Make a decision using VSIDS heuristic
    fn decide(&self) -> Option<(Variable, bool)> {
        let unassigned: Vec<Variable> = self
            .variables
            .iter()
            .filter(|&&var| !self.assignment.is_assigned(var))
            .copied()
            .collect();

        if unassigned.is_empty() {
            return None;
        }

        // Use VSIDS to pick variable
        let var = self
            .variable_activity
            .highest_activity(&unassigned)
            .unwrap_or(unassigned[0]);

        // Choose polarity (prefer positive for now)
        Some((var, true))
    }

    /// Check if all variables are assigned
    fn all_variables_assigned(&self) -> bool {
        self.variables
            .iter()
            .all(|&var| self.assignment.is_assigned(var))
    }

    /// Learn a new clause
    fn learn_clause(&mut self, clause: Clause) {
        self.stats.learned_clauses += 1;

        // Bump activity of variables in learned clause
        self.variable_activity.bump_clause(&clause);
        self.variable_activity.decay_all();

        // Add to learned clauses
        let clause_ref = clause_ref(clause);
        self.learned_clauses.push(clause_ref.clone());

        // Add to watch lists if not unit
        if clause_ref.len() >= 2 {
            let lit1 = clause_ref.literals()[0];
            let lit2 = clause_ref.literals()[1];

            // Watch for when these literals become false
            self.watch_lists
                .entry(lit1.negate())
                .or_default()
                .push(clause_ref.clone());
            self.watch_lists
                .entry(lit2.negate())
                .or_default()
                .push(clause_ref.clone());
        }
    }

    /// Backtrack to a given level
    fn backtrack_to(&mut self, level: u32) {
        // Clear propagation queue
        self.propagation_queue.clear();

        // Remove decisions above the level
        while let Some((var, _value)) = self.decisions.last() {
            if self.assignment.level(*var).unwrap_or(0) > level {
                self.decisions.pop();
            } else {
                break;
            }
        }

        // Backtrack assignment
        self.assignment.backtrack_to(level);

        // Clear conflict analysis
        self.conflict_analysis.clear();
    }

    /// Restart the search
    fn restart(&mut self) {
        self.backtrack_to(0);
        self.decisions.clear();
    }

    /// Analyze conflict for external explanation
    pub fn analyze_conflict(&self, problem: &DependencyProblem) -> ConflictExplanation {
        self.conflict_analysis.explain_unsat(problem)
    }
}

/// Result of unit propagation
#[derive(Debug)]
enum PropagationResult {
    Ok,
    Conflict(ClauseRef),
}

/// Result of watch update
#[derive(Debug)]
enum WatchResult {
    Ok,
    Unit(Literal),
    Conflict,
}

impl Default for SatSolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(i: u32) -> Variable {
        Variable::new(i)
    }

    fn pos(i: u32) -> Literal {
        Literal::positive(var(i))
    }

    fn neg(i: u32) -> Literal {
        Literal::negative(var(i))
    }

    #[test]
    fn test_simple_sat() {
        let mut solver = SatSolver::new();

        // (x0 ∨ x1) ∧ (¬x0 ∨ x2) ∧ (¬x1 ∨ ¬x2)
        solver.add_clause(Clause::new(vec![pos(0), pos(1)]));
        solver.add_clause(Clause::new(vec![neg(0), pos(2)]));
        solver.add_clause(Clause::new(vec![neg(1), neg(2)]));

        // Debug: print clauses
        println!("Clauses:");
        for (i, clause) in solver.clauses.iter().enumerate() {
            println!("  Clause {i}: {clause}");
        }

        let result = solver.solve();
        assert!(result.is_ok());

        let assignment = result.unwrap();

        // Debug: print the assignment
        println!(
            "Assignment: x0={}, x1={}, x2={}",
            assignment.is_true(var(0)),
            assignment.is_true(var(1)),
            assignment.is_true(var(2))
        );

        // Check that assignment satisfies all clauses
        assert!(assignment.is_true(var(0)) || assignment.is_true(var(1)));
        assert!(assignment.is_false(var(0)) || assignment.is_true(var(2)));
        assert!(assignment.is_false(var(1)) || assignment.is_false(var(2)));
    }

    #[tokio::test]
    async fn test_unsat() {
        let mut solver = SatSolver::new();

        // (x0) ∧ (¬x0)
        solver.add_clause(Clause::unit(pos(0)));
        solver.add_clause(Clause::unit(neg(0)));

        // Check that we have conflicting unit clauses in queue
        assert_eq!(solver.propagation_queue.len(), 2);

        let result = solver.solve();
        assert!(result.is_err());
    }

    #[test]
    fn test_unit_propagation() {
        let mut solver = SatSolver::new();

        // Unit clauses should propagate
        // (x0) ∧ (x0 ∨ x1) ∧ (¬x1 ∨ x2)
        solver.add_clause(Clause::unit(pos(0)));
        solver.add_clause(Clause::new(vec![pos(0), pos(1)]));
        solver.add_clause(Clause::new(vec![neg(1), pos(2)]));

        let result = solver.solve().unwrap();
        assert!(result.is_true(var(0)));
        // x1 can be either true or false, but if true then x2 must be true
        if result.is_true(var(1)) {
            assert!(result.is_true(var(2)));
        }
    }
}

//! Conflict analysis and learning for SAT solver

use super::{Assignment, Clause, ClauseRef, DependencyProblem, Literal, Variable};
use crate::sat::ConflictExplanation;
use std::collections::{HashMap, HashSet, VecDeque};

/// Conflict analysis data
#[derive(Debug, Clone)]
pub struct ConflictAnalysis {
    /// Implication graph: variable -> (implying clause, decision level)
    implication_graph: HashMap<Variable, (ClauseRef, u32)>,
    /// Conflict clause that caused the conflict
    conflict_clause: Option<ClauseRef>,
    /// Learned clauses from conflict analysis
    learned_clauses: Vec<Clause>,
}

impl ConflictAnalysis {
    /// Create new conflict analysis
    #[must_use]
    pub fn new() -> Self {
        Self {
            implication_graph: HashMap::new(),
            conflict_clause: None,
            learned_clauses: Vec::new(),
        }
    }

    /// Record an implication
    pub fn record_implication(&mut self, var: Variable, clause: ClauseRef, level: u32) {
        self.implication_graph.insert(var, (clause, level));
    }

    /// Set the conflict clause
    pub fn set_conflict(&mut self, clause: ClauseRef) {
        self.conflict_clause = Some(clause);
    }

    /// Analyze conflict and learn a new clause
    /// Returns the backtrack level
    pub fn analyze_conflict(&mut self, assignment: &Assignment) -> Option<(Clause, u32)> {
        let conflict_clause = self.conflict_clause.as_ref()?;
        let current_level = assignment.current_level();

        if current_level == 0 {
            // Conflict at level 0 means UNSAT
            return None;
        }

        // First UIP (Unique Implication Point) cut
        let mut learned_literals = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();
        let mut current_level_count = 0;

        // Start with conflict clause literals
        for &lit in conflict_clause.literals() {
            let var = lit.variable();
            if assignment.level(var).unwrap_or(0) == current_level {
                current_level_count += 1;
            }
            seen.insert(var);
            queue.push_back(var);
        }

        // Find first UIP
        while current_level_count > 1 {
            let Some(var) = queue.pop_front() else {
                break;
            };

            if let Some((clause, _level)) = self.implication_graph.get(&var) {
                // This variable was implied by a clause
                for &lit in clause.literals() {
                    let lit_var = lit.variable();
                    if lit_var != var && !seen.contains(&lit_var) {
                        seen.insert(lit_var);

                        let lit_level = assignment.level(lit_var).unwrap_or(0);
                        if lit_level == current_level {
                            current_level_count += 1;
                            queue.push_back(lit_var);
                        } else if lit_level > 0 {
                            // Add to learned clause (negated)
                            let assigned_value = assignment.is_true(lit_var);
                            let learned_lit = if assigned_value {
                                Literal::negative(lit_var)
                            } else {
                                Literal::positive(lit_var)
                            };
                            learned_literals.push(learned_lit);
                        }
                    }
                }

                if assignment.level(var).unwrap_or(0) == current_level {
                    current_level_count -= 1;
                }
            }
        }

        // Add the UIP literal
        if let Some(&uip_var) = queue.front() {
            let assigned_value = assignment.is_true(uip_var);
            let uip_lit = if assigned_value {
                Literal::negative(uip_var)
            } else {
                Literal::positive(uip_var)
            };
            learned_literals.push(uip_lit);
        }

        if learned_literals.is_empty() {
            return None;
        }

        // Find backtrack level (second highest level in learned clause)
        let mut levels: Vec<u32> = learned_literals
            .iter()
            .filter_map(|lit| assignment.level(lit.variable()))
            .collect();
        levels.sort_unstable();
        levels.dedup();

        let backtrack_level = if levels.len() > 1 {
            levels[levels.len() - 2]
        } else {
            0
        };

        let learned_clause = Clause::new(learned_literals);
        self.learned_clauses.push(learned_clause.clone());

        Some((learned_clause, backtrack_level))
    }

    /// Clear conflict analysis data
    pub fn clear(&mut self) {
        self.implication_graph.clear();
        self.conflict_clause = None;
    }

    /// Get learned clauses
    #[must_use]
    #[allow(dead_code)] // Used for debugging and testing
    pub fn learned_clauses(&self) -> &[Clause] {
        &self.learned_clauses
    }

    /// Analyze unsatisfiable problem and generate explanation
    pub fn explain_unsat(&self, problem: &DependencyProblem) -> ConflictExplanation {
        let mut conflicting_packages = Vec::new();
        let mut involved_packages = HashSet::new();

        // Analyze learned clauses to find conflicting packages
        for clause in &self.learned_clauses {
            let mut clause_packages = HashSet::new();

            for lit in clause.literals() {
                if let Some(package_version) = problem.variables.get_package(lit.variable()) {
                    clause_packages.insert(package_version.name.clone());
                    involved_packages.insert(package_version.name.clone());
                }
            }

            // If clause involves exactly 2 packages, they're likely conflicting
            if clause_packages.len() == 2 {
                let packages: Vec<String> = clause_packages.into_iter().collect();
                conflicting_packages.push((packages[0].clone(), packages[1].clone()));
            }
        }

        // Generate explanation message
        let message = if conflicting_packages.is_empty() {
            "Unable to find a valid set of package versions that satisfies all constraints"
                .to_string()
        } else {
            let conflicts: Vec<String> = conflicting_packages
                .iter()
                .map(|(p1, p2)| format!("{p1} and {p2}"))
                .collect();
            format!(
                "Dependency conflicts detected between: {}",
                conflicts.join(", ")
            )
        };

        // Generate suggestions
        let mut suggestions = Vec::new();

        if !involved_packages.is_empty() {
            suggestions.push(format!(
                "Try updating version constraints for: {}",
                involved_packages.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }

        suggestions.push(
            "Consider removing conflicting packages or finding compatible versions".to_string(),
        );

        ConflictExplanation::new(conflicting_packages, message, suggestions)
    }
}

impl Default for ConflictAnalysis {
    fn default() -> Self {
        Self::new()
    }
}

/// Variable activity scores for VSIDS heuristic
#[derive(Debug, Clone)]
pub struct VariableActivity {
    /// Activity score for each variable
    scores: HashMap<Variable, f64>,
    /// Decay factor (typically 0.95)
    decay: f64,
    /// Increment value
    increment: f64,
}

impl VariableActivity {
    /// Create new activity tracker
    #[must_use]
    pub fn new(decay: f64) -> Self {
        Self {
            scores: HashMap::new(),
            decay,
            increment: 1.0,
        }
    }

    /// Bump activity of variables in a clause
    pub fn bump_clause(&mut self, clause: &Clause) {
        for &lit in clause.literals() {
            self.bump_variable(lit.variable());
        }
    }

    /// Bump activity of a single variable
    pub fn bump_variable(&mut self, var: Variable) {
        let score = self.scores.entry(var).or_insert(0.0);
        *score += self.increment;
    }

    /// Decay all activities
    pub fn decay_all(&mut self) {
        self.increment /= self.decay;

        // Rescale if increment gets too large
        if self.increment > 1e100 {
            self.rescale();
        }
    }

    /// Rescale all scores to prevent overflow
    fn rescale(&mut self) {
        let scale = 1e-100;
        for score in self.scores.values_mut() {
            *score *= scale;
        }
        self.increment *= scale;
    }

    /// Get variable with highest activity
    #[must_use]
    pub fn highest_activity(&self, unassigned: &[Variable]) -> Option<Variable> {
        unassigned
            .iter()
            .max_by(|&a, &b| {
                let score_a = self.scores.get(a).copied().unwrap_or(0.0);
                let score_b = self.scores.get(b).copied().unwrap_or(0.0);
                score_a.partial_cmp(&score_b).unwrap()
            })
            .copied()
    }

    /// Get activity score for a variable
    #[must_use]
    #[allow(dead_code)] // Used for debugging and testing
    pub fn score(&self, var: Variable) -> f64 {
        self.scores.get(&var).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sat::clause::clause_ref;

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
    fn test_variable_activity() {
        let mut activity = VariableActivity::new(0.95);

        // Bump some variables
        activity.bump_variable(var(0));
        activity.bump_variable(var(0));
        activity.bump_variable(var(1));

        assert!(activity.score(var(0)) > activity.score(var(1)));
        assert!(activity.score(var(1)) > activity.score(var(2)));

        // Test highest activity
        let unassigned = vec![var(0), var(1), var(2)];
        assert_eq!(activity.highest_activity(&unassigned), Some(var(0)));

        // Test decay
        let score_before = activity.score(var(0));
        activity.decay_all();
        activity.bump_variable(var(0));
        let score_after = activity.score(var(0));
        assert!(score_after > score_before); // Due to increased increment
    }

    #[test]
    fn test_conflict_analysis_basic() {
        let mut analysis = ConflictAnalysis::new();
        let mut assignment = Assignment::new();

        // Create a simple conflict scenario
        // Clause: (x0 ∨ ¬x1)
        let clause = clause_ref(Clause::new(vec![pos(0), neg(1)]));

        // Assign both to make clause false
        assignment.assign(var(0), false, 1);
        assignment.assign(var(1), true, 2);

        // Record the conflict
        analysis.set_conflict(clause.clone());
        analysis.record_implication(var(1), clause, 2);

        // Analyze should find we need to backtrack
        let result = analysis.analyze_conflict(&assignment);
        assert!(result.is_some());

        let (learned, backtrack_level) = result.unwrap();
        assert!(backtrack_level < 2);
        assert!(!learned.is_empty());
    }
}

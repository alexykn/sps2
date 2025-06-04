//! SAT clause representation with two-watched literal optimization

use super::types::TruthValue;
use super::{Assignment, Literal, Variable};
use std::fmt;
use std::sync::Arc;

/// A clause in CNF (Conjunctive Normal Form)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    /// Literals in the clause
    literals: Vec<Literal>,
    /// Watched literal indices for two-watched literal scheme
    /// These are indices into the literals vector
    watched: Option<(usize, usize)>,
}

impl Clause {
    /// Create new clause from literals
    #[must_use]
    pub fn new(literals: Vec<Literal>) -> Self {
        let mut clause = Self {
            literals,
            watched: None,
        };

        // Initialize watched literals if clause has at least 2 literals
        if clause.literals.len() >= 2 {
            clause.watched = Some((0, 1));
        }

        clause
    }

    /// Create unit clause (single literal)
    #[must_use]
    pub fn unit(literal: Literal) -> Self {
        Self {
            literals: vec![literal],
            watched: None,
        }
    }

    /// Create binary clause (two literals)
    #[must_use]
    pub fn binary(lit1: Literal, lit2: Literal) -> Self {
        Self {
            literals: vec![lit1, lit2],
            watched: Some((0, 1)),
        }
    }

    /// Get literals in the clause
    #[must_use]
    pub fn literals(&self) -> &[Literal] {
        &self.literals
    }

    /// Get number of literals
    #[must_use]
    pub fn len(&self) -> usize {
        self.literals.len()
    }

    /// Check if clause is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.literals.is_empty()
    }

    /// Check if clause is unit (single literal)
    #[must_use]
    pub fn is_unit(&self) -> bool {
        self.literals.len() == 1
    }

    /// Check if clause is binary (two literals)
    #[must_use]
    pub fn is_binary(&self) -> bool {
        self.literals.len() == 2
    }

    /// Check if clause contains a literal
    #[must_use]
    pub fn contains(&self, literal: Literal) -> bool {
        self.literals.contains(&literal)
    }

    /// Check if clause contains a variable
    #[must_use]
    pub fn contains_variable(&self, var: Variable) -> bool {
        self.literals.iter().any(|lit| lit.variable() == var)
    }

    /// Evaluate clause under assignment
    #[must_use]
    pub fn evaluate(&self, assignment: &Assignment) -> TruthValue {
        let mut has_unassigned = false;

        for &lit in &self.literals {
            match assignment.eval_literal(lit) {
                TruthValue::True => return TruthValue::True, // Clause is satisfied
                TruthValue::Unassigned => has_unassigned = true,
                TruthValue::False => {} // Continue checking other literals
            }
        }

        if has_unassigned {
            TruthValue::Unassigned
        } else {
            TruthValue::False // All literals are false
        }
    }

    /// Check if clause is satisfied by assignment
    #[must_use]
    pub fn is_satisfied(&self, assignment: &Assignment) -> bool {
        self.evaluate(assignment).is_true()
    }

    /// Check if clause is conflicting (all literals false)
    #[must_use]
    pub fn is_conflict(&self, assignment: &Assignment) -> bool {
        self.evaluate(assignment).is_false()
    }

    /// Find unit literal if clause is unit under assignment
    /// Returns None if clause is not unit
    #[must_use]
    pub fn find_unit_literal(&self, assignment: &Assignment) -> Option<Literal> {
        let mut unassigned_literal = None;
        let mut unassigned_count = 0;

        for &lit in &self.literals {
            match assignment.eval_literal(lit) {
                TruthValue::True => return None, // Clause is already satisfied
                TruthValue::Unassigned => {
                    unassigned_literal = Some(lit);
                    unassigned_count += 1;
                    if unassigned_count > 1 {
                        return None; // More than one unassigned literal
                    }
                }
                TruthValue::False => {} // Continue
            }
        }

        unassigned_literal // Return the single unassigned literal if any
    }

    /// Get watched literals (for two-watched literal scheme)
    #[must_use]
    pub fn watched_literals(&self) -> Option<(Literal, Literal)> {
        self.watched
            .map(|(i, j)| (self.literals[i], self.literals[j]))
    }

    /// Update watched literals after assignment
    /// Returns true if watch was successfully updated, false if clause is unit or conflict
    pub fn update_watch(&mut self, assigned_lit: Literal, assignment: &Assignment) -> bool {
        let Some((w1, w2)) = self.watched else {
            return true; // No watched literals for unit clauses
        };

        // Check if assigned literal is one of the watched
        let assigned_idx = if self.literals[w1] == assigned_lit {
            Some(w1)
        } else if self.literals[w2] == assigned_lit {
            Some(w2)
        } else {
            return true; // Assigned literal is not watched
        };

        let Some(assigned_watch_idx) = assigned_idx else {
            return true;
        };

        // Check if assigned literal is false
        if assignment.eval_literal(assigned_lit).is_true() {
            return true; // Clause is satisfied
        }

        // Find new literal to watch
        for (i, &lit) in self.literals.iter().enumerate() {
            if i == w1 || i == w2 {
                continue; // Skip current watched literals
            }

            let lit_value = assignment.eval_literal(lit);
            if !lit_value.is_false() {
                // Found non-false literal, update watch
                if assigned_watch_idx == w1 {
                    self.watched = Some((i, w2));
                } else {
                    self.watched = Some((w1, i));
                }
                return true;
            }
        }

        // No replacement found - clause is unit or conflict
        false
    }

    /// Get variables in the clause
    pub fn variables(&self) -> impl Iterator<Item = Variable> + '_ {
        self.literals.iter().map(|lit| lit.variable())
    }

    /// Remove duplicate literals and check for tautology
    /// Returns None if clause is tautology (contains both x and ¬x)
    #[must_use]
    pub fn simplify(mut self) -> Option<Self> {
        // Remove duplicates
        self.literals
            .sort_by_key(|lit| (lit.variable().index(), lit.is_positive()));
        self.literals.dedup();

        // Check for tautology
        for i in 0..self.literals.len() {
            for j in (i + 1)..self.literals.len() {
                if self.literals[i].variable() == self.literals[j].variable()
                    && self.literals[i].is_positive() != self.literals[j].is_positive()
                {
                    return None; // Tautology
                }
            }
        }

        // Re-initialize watched literals
        if self.literals.len() >= 2 {
            self.watched = Some((0, 1));
        } else {
            self.watched = None;
        }

        Some(self)
    }
}

impl fmt::Display for Clause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            write!(f, "⊥") // Empty clause (false)
        } else {
            let literals: Vec<String> = self.literals.iter().map(ToString::to_string).collect();
            write!(f, "({})", literals.join(" ∨ "))
        }
    }
}

/// Reference to a clause (for efficient storage)
pub type ClauseRef = Arc<Clause>;

/// Create a clause reference
#[must_use]
pub fn clause_ref(clause: Clause) -> ClauseRef {
    Arc::new(clause)
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
    fn test_clause_creation() {
        let unit = Clause::unit(pos(0));
        assert!(unit.is_unit());
        assert_eq!(unit.len(), 1);

        let binary = Clause::binary(pos(0), neg(1));
        assert!(binary.is_binary());
        assert_eq!(binary.len(), 2);

        let clause = Clause::new(vec![pos(0), neg(1), pos(2)]);
        assert_eq!(clause.len(), 3);
        assert!(!clause.is_unit());
        assert!(!clause.is_binary());
    }

    #[test]
    fn test_clause_evaluation() {
        let clause = Clause::new(vec![pos(0), neg(1), pos(2)]);
        let mut assignment = Assignment::new();

        // All unassigned
        assert_eq!(clause.evaluate(&assignment), TruthValue::Unassigned);

        // Make one literal true
        assignment.assign(var(0), true, 1);
        assert_eq!(clause.evaluate(&assignment), TruthValue::True);
        assert!(clause.is_satisfied(&assignment));

        // Make all literals false
        assignment.clear();
        assignment.assign(var(0), false, 1);
        assignment.assign(var(1), true, 1);
        assignment.assign(var(2), false, 1);
        assert_eq!(clause.evaluate(&assignment), TruthValue::False);
        assert!(clause.is_conflict(&assignment));
    }

    #[test]
    fn test_unit_propagation() {
        let clause = Clause::new(vec![pos(0), neg(1), pos(2)]);
        let mut assignment = Assignment::new();

        // No unit literal when unassigned
        assert_eq!(clause.find_unit_literal(&assignment), None);

        // Make two literals false
        assignment.assign(var(0), false, 1);
        assignment.assign(var(1), true, 1);

        // Now pos(2) is unit literal
        assert_eq!(clause.find_unit_literal(&assignment), Some(pos(2)));

        // Satisfy the clause
        assignment.assign(var(2), true, 1);
        assert_eq!(clause.find_unit_literal(&assignment), None);
    }

    #[test]
    fn test_simplify() {
        // Test duplicate removal
        let clause = Clause::new(vec![pos(0), pos(1), pos(0)]);
        let simplified = clause.simplify().unwrap();
        assert_eq!(simplified.len(), 2);

        // Test tautology detection
        let tautology = Clause::new(vec![pos(0), neg(0)]);
        assert!(tautology.simplify().is_none());
    }

    #[test]
    fn test_watched_literals() {
        let mut clause = Clause::new(vec![pos(0), neg(1), pos(2)]);
        let mut assignment = Assignment::new();

        // Initial watched literals
        assert_eq!(clause.watched_literals(), Some((pos(0), neg(1))));

        // Assign first watched literal to false
        assignment.assign(var(0), false, 1);
        assert!(clause.update_watch(pos(0), &assignment));

        // Watch should have moved to include pos(2)
        let watched = clause.watched_literals().unwrap();
        assert!(watched.0 == pos(2) || watched.1 == pos(2));
    }
}

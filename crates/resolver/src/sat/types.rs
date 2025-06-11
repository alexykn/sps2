//! Basic types for SAT solving

use std::collections::HashMap;
use std::fmt;

/// A boolean variable in the SAT problem
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Variable(pub u32);

impl Variable {
    /// Create new variable with given index
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Get the variable index
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// A literal is a variable or its negation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Literal {
    variable: Variable,
    positive: bool,
}

impl Literal {
    /// Create a positive literal
    #[must_use]
    pub const fn positive(variable: Variable) -> Self {
        Self {
            variable,
            positive: true,
        }
    }

    /// Create a negative literal
    #[must_use]
    pub const fn negative(variable: Variable) -> Self {
        Self {
            variable,
            positive: false,
        }
    }

    /// Get the variable
    #[must_use]
    pub const fn variable(self) -> Variable {
        self.variable
    }

    /// Check if literal is positive
    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.positive
    }

    /// Check if literal is negative
    #[must_use]
    pub const fn is_negative(self) -> bool {
        !self.positive
    }

    /// Negate the literal
    #[must_use]
    pub const fn negate(self) -> Self {
        Self {
            variable: self.variable,
            positive: !self.positive,
        }
    }

    /// Convert to DIMACS format integer
    /// Positive literals are variable index + 1
    /// Negative literals are -(variable index + 1)
    #[must_use]
    pub fn to_dimacs(self) -> i32 {
        let index = self.variable.index().min(i32::MAX as u32 - 1) + 1;
        let index_i32 = i32::try_from(index).expect("Variable index too large for DIMACS format");
        if self.positive {
            index_i32
        } else {
            -index_i32
        }
    }

    /// Create from DIMACS format integer
    #[must_use]
    pub fn from_dimacs(dimacs: i32) -> Self {
        let index = dimacs.unsigned_abs() - 1;
        if dimacs > 0 {
            Self::positive(Variable::new(index))
        } else {
            Self::negative(Variable::new(index))
        }
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.positive {
            write!(f, "{}", self.variable)
        } else {
            write!(f, "¬{}", self.variable)
        }
    }
}

/// Truth value for a variable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruthValue {
    True,
    False,
    Unassigned,
}

impl TruthValue {
    /// Check if value is assigned
    #[must_use]
    pub const fn is_assigned(self) -> bool {
        matches!(self, Self::True | Self::False)
    }

    /// Check if value is true
    #[must_use]
    pub const fn is_true(self) -> bool {
        matches!(self, Self::True)
    }

    /// Check if value is false
    #[must_use]
    pub const fn is_false(self) -> bool {
        matches!(self, Self::False)
    }

    /// Convert to boolean (panics if unassigned)
    #[must_use]
    pub fn to_bool(self) -> bool {
        match self {
            Self::True => true,
            Self::False => false,
            Self::Unassigned => panic!("Cannot convert unassigned value to bool"),
        }
    }
}

/// Variable assignment
#[derive(Debug, Clone)]
pub struct Assignment {
    /// Current assignment of variables
    values: HashMap<Variable, TruthValue>,
    /// Decision level for each assigned variable
    levels: HashMap<Variable, u32>,
    /// Order in which variables were assigned
    trail: Vec<Variable>,
    /// Current decision level
    current_level: u32,
}

impl Assignment {
    /// Create new empty assignment
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            levels: HashMap::new(),
            trail: Vec::new(),
            current_level: 0,
        }
    }

    /// Get value of a variable
    #[must_use]
    pub fn get(&self, var: Variable) -> TruthValue {
        self.values
            .get(&var)
            .copied()
            .unwrap_or(TruthValue::Unassigned)
    }

    /// Check if variable is assigned
    #[must_use]
    pub fn is_assigned(&self, var: Variable) -> bool {
        self.get(var).is_assigned()
    }

    /// Check if variable is true
    #[must_use]
    pub fn is_true(&self, var: Variable) -> bool {
        self.get(var).is_true()
    }

    /// Check if variable is false
    #[must_use]
    pub fn is_false(&self, var: Variable) -> bool {
        self.get(var).is_false()
    }

    /// Evaluate a literal under this assignment
    #[must_use]
    pub fn eval_literal(&self, lit: Literal) -> TruthValue {
        match self.get(lit.variable()) {
            TruthValue::Unassigned => TruthValue::Unassigned,
            TruthValue::True => {
                if lit.is_positive() {
                    TruthValue::True
                } else {
                    TruthValue::False
                }
            }
            TruthValue::False => {
                if lit.is_positive() {
                    TruthValue::False
                } else {
                    TruthValue::True
                }
            }
        }
    }

    /// Assign a variable
    pub fn assign(&mut self, var: Variable, value: bool, level: u32) {
        let truth_value = if value {
            TruthValue::True
        } else {
            TruthValue::False
        };
        self.values.insert(var, truth_value);
        self.levels.insert(var, level);
        self.trail.push(var);
        self.current_level = level;
    }

    /// Unassign a variable
    pub fn unassign(&mut self, var: Variable) {
        self.values.remove(&var);
        self.levels.remove(&var);
        self.trail.retain(|&v| v != var);
    }

    /// Get decision level of a variable
    #[must_use]
    pub fn level(&self, var: Variable) -> Option<u32> {
        self.levels.get(&var).copied()
    }

    /// Get current decision level
    #[must_use]
    pub const fn current_level(&self) -> u32 {
        self.current_level
    }

    /// Backtrack to a given level
    pub fn backtrack_to(&mut self, level: u32) {
        // Remove all assignments at higher levels
        self.trail.retain(|&var| {
            if let Some(&var_level) = self.levels.get(&var) {
                if var_level > level {
                    self.values.remove(&var);
                    self.levels.remove(&var);
                    false
                } else {
                    true
                }
            } else {
                false
            }
        });

        self.current_level = level;
    }

    /// Get number of assigned variables
    #[must_use]
    pub fn num_assigned(&self) -> usize {
        self.values.len()
    }

    /// Check if all variables in a set are assigned
    #[must_use]
    pub fn all_assigned(&self, vars: &[Variable]) -> bool {
        vars.iter().all(|&v| self.is_assigned(v))
    }

    /// Get trail of assignments
    #[must_use]
    pub fn trail(&self) -> &[Variable] {
        &self.trail
    }

    /// Clear all assignments
    pub fn clear(&mut self) {
        self.values.clear();
        self.levels.clear();
        self.trail.clear();
        self.current_level = 0;
    }
}

impl Default for Assignment {
    fn default() -> Self {
        Self::new()
    }
}

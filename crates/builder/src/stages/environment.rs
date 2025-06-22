//! Environment stage types and operations

use crate::environment::IsolationLevel;
use serde::{Deserialize, Serialize};

/// Environment setup operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EnvironmentStep {
    /// Set isolation level
    SetIsolation { level: IsolationLevel },

    /// Apply compiler defaults
    WithDefaults,

    /// Allow network access
    AllowNetwork { enabled: bool },

    /// Set environment variable
    SetEnv { key: String, value: String },
}

// Note: ParsedEnvironment is recipe::model::Environment

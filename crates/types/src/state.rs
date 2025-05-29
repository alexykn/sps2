//! State management type definitions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// State identifier
pub type StateId = Uuid;

/// Information about a system state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateInfo {
    pub id: StateId,
    pub parent: Option<StateId>,
    pub timestamp: DateTime<Utc>,
    pub operation: String,
    pub package_count: usize,
    pub total_size: u64,
}

/// State transition record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransition {
    pub from: StateId,
    pub to: StateId,
    pub operation: String,
    pub timestamp: DateTime<Utc>,
    pub success: bool,
    pub rollback_of: Option<StateId>,
}

//! State management type definitions

use crate::Version;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// State identifier
pub type StateId = Uuid;

/// Information about a system state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateInfo {
    /// State ID
    pub id: StateId,
    /// Parent state ID
    pub parent: Option<StateId>,
    /// Parent state ID (alternative field name)
    pub parent_id: Option<StateId>,
    /// Creation timestamp
    pub timestamp: DateTime<Utc>,
    /// Operation that created this state
    pub operation: String,
    /// Whether this is the current state
    pub current: bool,
    /// Number of packages in this state
    pub package_count: usize,
    /// Total size of packages
    pub total_size: u64,
    /// Summary of changes from parent (using `ops::OpChange` for change type info)
    pub changes: Vec<OpChange>,
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

/// Operation change for state tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpChange {
    /// Change type
    pub change_type: ChangeType,
    /// Package name
    pub package: String,
    /// Old version (for updates/removals)
    pub old_version: Option<Version>,
    /// New version (for installs/updates)
    pub new_version: Option<Version>,
}

/// Type of operation change
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    /// Package was installed
    Install,
    /// Package was updated
    Update,
    /// Package was removed
    Remove,
    /// Package was downgraded
    Downgrade,
}

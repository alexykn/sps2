//! State management type definitions

use crate::Version;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use uuid::Uuid;

/// State identifier
pub type StateId = Uuid;

/// Information about a system state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateInfo {
    /// State ID
    pub id: StateId,
    /// Parent state ID
    #[serde(alias = "parent_id")]
    pub parent: Option<StateId>,
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

/// Identifier for the live slot containing a state snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SlotId {
    /// Primary slot (`live-A`).
    A,
    /// Secondary slot (`live-B`).
    B,
}

impl SlotId {
    /// All available slots.
    pub const ALL: [SlotId; 2] = [SlotId::A, SlotId::B];

    /// Directory name associated with the slot.
    #[must_use]
    pub fn dir_name(self) -> &'static str {
        match self {
            SlotId::A => "live-A",
            SlotId::B => "live-B",
        }
    }

    /// Return the opposite slot.
    #[must_use]
    pub fn other(self) -> SlotId {
        match self {
            SlotId::A => SlotId::B,
            SlotId::B => SlotId::A,
        }
    }
}

impl Default for SlotId {
    fn default() -> Self {
        SlotId::A
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.dir_name())
    }
}

impl Serialize for SlotId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.dir_name())
    }
}

impl<'de> Deserialize<'de> for SlotId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "live-A" | "A" | "a" => Ok(SlotId::A),
            "live-B" | "B" | "b" => Ok(SlotId::B),
            other => Err(serde::de::Error::custom(format!(
                "unknown slot identifier: {other}"
            ))),
        }
    }
}

/// Phase of a two-phase commit transaction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransactionPhase {
    /// The database changes are committed, and the system is ready for the filesystem swap
    Prepared,
    /// Filesystem swap has been executed
    Swapped,
}

/// Transaction journal for crash recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionJournal {
    /// New state ID being transitioned to
    pub new_state_id: Uuid,
    /// Parent state ID we're transitioning from
    pub parent_state_id: Uuid,
    /// Path to the staging directory
    pub staging_path: std::path::PathBuf,
    /// Slot containing the prepared state
    #[serde(default)]
    pub staging_slot: SlotId,
    /// Current phase of the transaction
    pub phase: TransactionPhase,
    /// Operation type (install, uninstall, rollback, etc.)
    pub operation: String,
}

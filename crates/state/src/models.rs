//! Database models for state management

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use spsv2_hash::Hash;
use spsv2_types::{StateId, Version};
use sqlx::FromRow;

/// A system state record
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct State {
    pub id: String,
    pub parent_id: Option<String>,
    pub created_at: i64,
    pub operation: String,
    pub success: bool,
    pub rollback_of: Option<String>,
}

impl State {
    /// Convert to `StateId`
    ///
    /// # Panics
    ///
    /// Panics if the stored ID is not a valid UUID.
    #[must_use]
    pub fn state_id(&self) -> StateId {
        uuid::Uuid::parse_str(&self.id).expect("valid UUID in database")
    }

    /// Get creation timestamp
    ///
    /// # Panics
    ///
    /// Panics if the stored timestamp is not valid.
    #[must_use]
    pub fn timestamp(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.created_at, 0).expect("valid timestamp in database")
    }
}

/// An installed package record
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Package {
    pub id: i64,
    pub state_id: String,
    pub name: String,
    pub version: String,
    pub hash: String,
    pub size: i64,
    pub installed_at: i64,
}

impl Package {
    /// Parse the version
    ///
    /// # Panics
    ///
    /// Panics if the stored version string is not valid.
    #[must_use]
    pub fn version(&self) -> Version {
        Version::parse(&self.version).expect("valid version in database")
    }

    /// Parse the hash
    ///
    /// # Panics
    ///
    /// Panics if the stored hash string is not valid.
    #[must_use]
    pub fn hash(&self) -> Hash {
        Hash::from_hex(&self.hash).expect("valid hash in database")
    }
}

/// A package dependency record
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Dependency {
    pub id: i64,
    pub package_id: i64,
    pub dep_name: String,
    pub dep_spec: String,
    pub dep_kind: String,
}

/// A store reference count record
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct StoreRef {
    pub hash: String,
    pub ref_count: i64,
    pub size: i64,
    pub created_at: i64,
}

impl StoreRef {
    /// Parse the hash
    ///
    /// # Panics
    ///
    /// Panics if the stored hash string is not valid.
    #[must_use]
    pub fn hash(&self) -> Hash {
        Hash::from_hex(&self.hash).expect("valid hash in database")
    }
}

/// Package reference for state transitions
#[derive(Debug, Clone)]
pub struct PackageRef {
    pub state_id: uuid::Uuid,
    pub package_id: spsv2_resolver::PackageId,
}

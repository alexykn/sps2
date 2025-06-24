//! State verification guard for ensuring database/filesystem consistency

use sps2_errors::{Error, OpsError};
use sps2_events::EventSender;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use uuid::Uuid;

/// Verification level for state checking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationLevel {
    /// Quick check - file existence only
    Quick,
    /// Standard check - existence + metadata
    Standard,
    /// Full check - existence + metadata + content hash
    Full,
}

impl Default for VerificationLevel {
    fn default() -> Self {
        Self::Standard
    }
}

/// Type of discrepancy found during verification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discrepancy {
    /// File expected but not found
    MissingFile {
        package_name: String,
        package_version: String,
        file_path: String,
    },
    /// File exists but has wrong type (file vs directory)
    TypeMismatch {
        package_name: String,
        package_version: String,
        file_path: String,
        expected_directory: bool,
        actual_directory: bool,
    },
    /// File content doesn't match expected hash
    CorruptedFile {
        package_name: String,
        package_version: String,
        file_path: String,
        expected_hash: String,
        actual_hash: String,
    },
    /// File exists but not tracked in database
    OrphanedFile { file_path: String },
    /// Python virtual environment missing
    MissingVenv {
        package_name: String,
        package_version: String,
        venv_path: String,
    },
}

/// Result of verification check
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// State ID that was verified
    pub state_id: Uuid,
    /// List of discrepancies found
    pub discrepancies: Vec<Discrepancy>,
    /// Whether verification passed (no discrepancies)
    pub is_valid: bool,
    /// Time taken for verification in milliseconds
    pub duration_ms: u64,
}

impl VerificationResult {
    /// Create a new verification result
    pub fn new(state_id: Uuid, discrepancies: Vec<Discrepancy>, duration_ms: u64) -> Self {
        let is_valid = discrepancies.is_empty();
        Self {
            state_id,
            discrepancies,
            is_valid,
            duration_ms,
        }
    }
}

/// State verification guard for consistency checking
pub struct StateVerificationGuard {
    /// State manager for database operations
    state_manager: StateManager,
    /// Package store for content verification
    store: PackageStore,
    /// Event sender for progress reporting
    tx: EventSender,
    /// Verification level
    level: VerificationLevel,
}

impl StateVerificationGuard {
    /// Create a new verification guard with builder
    pub fn builder() -> StateVerificationGuardBuilder {
        StateVerificationGuardBuilder::new()
    }

    /// Verify current state and optionally heal discrepancies
    pub async fn verify_and_heal(&self) -> Result<VerificationResult, Error> {
        // TODO: Implement verification logic in OPS-43
        let state_id = self.state_manager.get_active_state().await?;
        Ok(VerificationResult::new(state_id, vec![], 0))
    }

    /// Verify current state without healing
    pub async fn verify_only(&self) -> Result<VerificationResult, Error> {
        // TODO: Implement verification logic in OPS-43
        let state_id = self.state_manager.get_active_state().await?;
        Ok(VerificationResult::new(state_id, vec![], 0))
    }

    /// Get the current verification level
    pub fn level(&self) -> VerificationLevel {
        self.level
    }
}

/// Builder for StateVerificationGuard
pub struct StateVerificationGuardBuilder {
    state_manager: Option<StateManager>,
    store: Option<PackageStore>,
    tx: Option<EventSender>,
    level: VerificationLevel,
}

impl StateVerificationGuardBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            state_manager: None,
            store: None,
            tx: None,
            level: VerificationLevel::default(),
        }
    }

    /// Set the state manager
    pub fn with_state_manager(mut self, state_manager: StateManager) -> Self {
        self.state_manager = Some(state_manager);
        self
    }

    /// Set the package store
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Set the event sender
    pub fn with_event_sender(mut self, tx: EventSender) -> Self {
        self.tx = Some(tx);
        self
    }

    /// Set the verification level
    pub fn with_level(mut self, level: VerificationLevel) -> Self {
        self.level = level;
        self
    }

    /// Build the guard
    pub fn build(self) -> Result<StateVerificationGuard, Error> {
        let state_manager = self
            .state_manager
            .ok_or_else(|| OpsError::MissingComponent {
                component: "StateManager".to_string(),
            })?;

        let store = self.store.ok_or_else(|| OpsError::MissingComponent {
            component: "PackageStore".to_string(),
        })?;

        let tx = self.tx.ok_or_else(|| OpsError::MissingComponent {
            component: "EventSender".to_string(),
        })?;

        Ok(StateVerificationGuard {
            state_manager,
            store,
            tx,
            level: self.level,
        })
    }
}

impl Default for StateVerificationGuardBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_level_default() {
        assert_eq!(VerificationLevel::default(), VerificationLevel::Standard);
    }

    #[test]
    fn test_verification_result_validity() {
        let state_id = Uuid::new_v4();

        // Test valid result (no discrepancies)
        let result = VerificationResult::new(state_id, vec![], 100);
        assert!(result.is_valid);
        assert_eq!(result.discrepancies.len(), 0);
        assert_eq!(result.duration_ms, 100);

        // Test invalid result (with discrepancies)
        let discrepancies = vec![Discrepancy::MissingFile {
            package_name: "test".to_string(),
            package_version: "1.0.0".to_string(),
            file_path: "/bin/test".to_string(),
        }];
        let result = VerificationResult::new(state_id, discrepancies, 200);
        assert!(!result.is_valid);
        assert_eq!(result.discrepancies.len(), 1);
    }

    #[test]
    fn test_builder_pattern() {
        // Test that builder requires all fields
        let builder = StateVerificationGuardBuilder::new();
        assert!(builder.build().is_err());

        // Test builder with verification level
        let builder = StateVerificationGuardBuilder::new().with_level(VerificationLevel::Full);
        assert!(builder.build().is_err()); // Still missing required fields
    }
}

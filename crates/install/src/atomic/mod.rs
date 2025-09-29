//! Atomic installation operations using APFS clonefile and state transitions
//!
//! This module provides atomic installation capabilities with:
//! - APFS-optimized file operations for instant, space-efficient copies
//! - Hard link creation for efficient package linking
//! - State transitions with rollback support
//! - Platform-specific filesystem optimizations

pub mod fs;
pub mod installer;
pub mod package;
pub mod transition;

// Re-export main public API
pub use installer::AtomicInstaller;
pub use transition::StateTransition;

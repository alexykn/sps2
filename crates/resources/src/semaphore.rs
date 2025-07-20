//! Semaphore utilities for resource management
//!
//! This module provides helper functions for managing semaphores with
//! consistent error handling across the sps2 package manager.

use sps2_errors::{Error, InstallError};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Acquire a semaphore permit with proper error handling
///
/// This helper function provides consistent error handling for semaphore
/// acquisition across all modules in sps2.
///
/// # Arguments
///
/// * `semaphore` - The semaphore to acquire a permit from
/// * `operation` - Description of the operation for error reporting
///
/// # Errors
///
/// Returns an error if the semaphore is closed or acquisition fails
pub async fn acquire_semaphore_permit(
    semaphore: Arc<Semaphore>,
    operation: &str,
) -> Result<OwnedSemaphorePermit, Error> {
    semaphore.clone().acquire_owned().await.map_err(|_| {
        InstallError::ConcurrencyError {
            message: format!("failed to acquire semaphore for {operation}"),
        }
        .into()
    })
}

/// Try to acquire a semaphore permit without waiting
///
/// This helper function attempts to acquire a semaphore permit immediately
/// without blocking. Useful for checking resource availability.
///
/// # Arguments
///
/// * `semaphore` - The semaphore to try to acquire a permit from
///
/// # Returns
///
/// Returns `Ok(Some(permit))` if successful, `Ok(None)` if would block,
/// or an error if the semaphore is closed.
///
/// # Errors
///
/// Returns an error if the semaphore is closed.
pub fn try_acquire_semaphore_permit(
    semaphore: &Arc<Semaphore>,
) -> Result<Option<OwnedSemaphorePermit>, Error> {
    match semaphore.clone().try_acquire_owned() {
        Ok(permit) => Ok(Some(permit)),
        Err(tokio::sync::TryAcquireError::NoPermits) => Ok(None),
        Err(tokio::sync::TryAcquireError::Closed) => Err(InstallError::ConcurrencyError {
            message: "semaphore is closed".to_string(),
        }
        .into()),
    }
}

/// Create a semaphore with a specified number of permits
///
/// This is a convenience function for creating semaphores with consistent
/// error handling and documentation.
///
/// # Arguments
///
/// * `permits` - Number of permits the semaphore should have
///
/// # Returns
///
/// Returns an Arc-wrapped semaphore for shared ownership
#[must_use]
pub fn create_semaphore(permits: usize) -> Arc<Semaphore> {
    Arc::new(Semaphore::new(permits))
}

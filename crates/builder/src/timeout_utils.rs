//! Timeout utilities for build operations

use sps2_errors::{BuildError, Error};
use std::future::Future;
use std::time::Duration;

/// Execute a future with a timeout
pub async fn with_timeout<T, F>(
    future: F,
    timeout_seconds: u64,
    package_name: &str,
) -> Result<T, Error>
where
    F: Future<Output = Result<T, Error>>,
{
    tokio::time::timeout(Duration::from_secs(timeout_seconds), future)
        .await
        .map_err(|_| -> Error {
            BuildError::BuildTimeout {
                package: package_name.to_string(),
                timeout_seconds,
            }
            .into()
        })?
}

/// Execute a future with an optional timeout
pub async fn with_optional_timeout<T, F>(
    future: F,
    timeout_seconds: Option<u64>,
    package_name: &str,
) -> Result<T, Error>
where
    F: Future<Output = Result<T, Error>>,
{
    if let Some(timeout) = timeout_seconds {
        with_timeout(future, timeout, package_name).await
    } else {
        future.await
    }
}

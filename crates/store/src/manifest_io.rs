#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Manifest I/O helpers colocated with the store.

use sps2_errors::{Error, PackageError};
use sps2_types::Manifest;
use std::path::Path;

/// Read `manifest.toml` from a path
///
/// # Errors
/// Returns an error if reading or parsing the manifest fails.
pub async fn read_manifest(path: &Path) -> Result<Manifest, Error> {
    let content =
        tokio::fs::read_to_string(path)
            .await
            .map_err(|e| PackageError::InvalidManifest {
                message: format!("failed to read manifest: {e}"),
            })?;
    Manifest::from_toml(&content)
}

/// Write `manifest.toml` to a path
///
/// # Errors
/// Returns an error if serialization or writing fails.
pub async fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), Error> {
    let content = manifest.to_toml()?;
    Ok(tokio::fs::write(path, content)
        .await
        .map_err(|e| PackageError::InvalidManifest {
            message: format!("failed to write manifest: {e}"),
        })?)
}

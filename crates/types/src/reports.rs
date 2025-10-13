//! Report type definitions for operations

use crate::Version;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Installation report
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallReport {
    /// Packages that were installed
    pub installed: Vec<PackageChange>,
    /// Packages that were updated
    pub updated: Vec<PackageChange>,
    /// Packages that were removed
    pub removed: Vec<PackageChange>,
    /// New state ID
    pub state_id: Uuid,
    /// Total execution time
    pub duration_ms: u64,
}

/// Build report
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildReport {
    /// Package that was built
    pub package: String,
    /// Version that was built
    pub version: Version,
    /// Output file path
    pub output_path: PathBuf,
    /// Build duration
    pub duration_ms: u64,
    /// Whether SBOM was generated (currently unused - SBOM soft-disabled)
    pub sbom_generated: bool,
}

/// Package change for reports
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageChange {
    /// Package name
    pub name: String,
    /// Previous version
    pub from_version: Option<Version>,
    /// New version
    pub to_version: Option<Version>,
    /// Size in bytes
    pub size: Option<u64>,
}

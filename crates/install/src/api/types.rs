use sps2_hash::Hash;
use std::path::PathBuf;

/// Prepared package data passed from ParallelExecutor to AtomicInstaller
///
/// This structure contains all the information needed by AtomicInstaller
/// to install a package without having to look up package_map or perform
/// additional database queries.
#[derive(Clone, Debug)]
pub struct PreparedPackage {
    /// Package hash
    pub hash: Hash,
    /// Package size in bytes
    pub size: u64,
    /// Path to the package in the store
    pub store_path: PathBuf,
    /// Whether this package was downloaded or local
    pub is_local: bool,
    /// Optional package archive hash (BLAKE3) provided by the repository
    pub package_hash: Option<Hash>,
}

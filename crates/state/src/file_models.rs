//! Database models for file-level content addressable storage

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sps2_hash::Hash;
use sqlx::FromRow;

/// A file object in content-addressed storage
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct FileObject {
    pub hash: String,
    pub size: i64,
    pub created_at: i64,
    pub ref_count: i64,
    pub is_executable: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
}

impl FileObject {
    /// Parse the hash
    ///
    /// # Panics
    ///
    /// Panics if the stored hash string is not valid.
    #[must_use]
    pub fn hash(&self) -> Hash {
        Hash::from_hex(&self.hash).expect("valid hash in database")
    }

    /// Get creation timestamp
    ///
    /// # Panics
    ///
    /// Panics if the stored timestamp is not valid.
    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.created_at, 0).expect("valid timestamp in database")
    }

    /// Check if the file is referenced by any packages
    #[must_use]
    pub fn is_referenced(&self) -> bool {
        self.ref_count > 0
    }
}

/// A file entry within a package
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PackageFileEntry {
    pub id: i64,
    pub package_id: i64,
    pub file_hash: String,
    pub relative_path: String,
    pub permissions: i64,
    pub uid: i64,
    pub gid: i64,
    pub mtime: Option<i64>,
}

impl PackageFileEntry {
    /// Parse the file hash
    ///
    /// # Panics
    ///
    /// Panics if the stored hash string is not valid.
    #[must_use]
    pub fn file_hash(&self) -> Hash {
        Hash::from_hex(&self.file_hash).expect("valid hash in database")
    }

    /// Get modification timestamp if available
    #[must_use]
    pub fn mtime(&self) -> Option<DateTime<Utc>> {
        self.mtime.and_then(|ts| DateTime::from_timestamp(ts, 0))
    }

    /// Get Unix permissions as octal
    #[must_use]
    pub fn permissions_octal(&self) -> u32 {
        self.permissions as u32
    }
}

/// An installed file tracking its location
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct InstalledFile {
    pub id: i64,
    pub state_id: String,
    pub package_id: i64,
    pub file_hash: String,
    pub installed_path: String,
    pub is_directory: bool,
}

impl InstalledFile {
    /// Parse the file hash
    ///
    /// # Panics
    ///
    /// Panics if the stored hash string is not valid.
    #[must_use]
    pub fn file_hash(&self) -> Hash {
        Hash::from_hex(&self.file_hash).expect("valid hash in database")
    }

    /// Parse the state ID as UUID
    ///
    /// # Panics
    ///
    /// Panics if the stored state ID is not a valid UUID.
    #[must_use]
    pub fn state_uuid(&self) -> uuid::Uuid {
        uuid::Uuid::parse_str(&self.state_id).expect("valid UUID in database")
    }
}

/// Simple modification time tracker for file verification optimization
#[derive(Debug, Clone)]
pub struct FileMTimeTracker {
    pub file_path: String,
    pub last_verified_mtime: i64,
}

/// Summary statistics for file-level storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStorageStats {
    pub total_files: i64,
    pub unique_files: i64,
    pub total_size: i64,
    pub deduplicated_size: i64,
    pub deduplication_ratio: f64,
}

/// File metadata for storage operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub size: i64,
    pub permissions: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime: Option<i64>,
    pub is_executable: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
}

impl FileMetadata {
    /// Create metadata for a regular file
    #[must_use]
    pub fn regular_file(size: i64, permissions: u32) -> Self {
        Self {
            size,
            permissions,
            uid: 0,
            gid: 0,
            mtime: None,
            is_executable: permissions & 0o111 != 0,
            is_symlink: false,
            symlink_target: None,
        }
    }

    /// Create metadata for a symlink
    #[must_use]
    pub fn symlink(target: String) -> Self {
        Self {
            size: target.len() as i64,
            permissions: 0o777,
            uid: 0,
            gid: 0,
            mtime: None,
            is_executable: false,
            is_symlink: true,
            symlink_target: Some(target),
        }
    }
}

/// Result of a file deduplication check
#[derive(Debug, Clone)]
pub struct DeduplicationResult {
    pub hash: Hash,
    pub was_duplicate: bool,
    pub ref_count: i64,
    pub space_saved: i64,
}

/// File reference for batch operations
#[derive(Debug, Clone)]
pub struct FileReference {
    pub package_id: i64,
    pub relative_path: String,
    pub hash: Hash,
    pub metadata: FileMetadata,
}

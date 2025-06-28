#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! BLAKE3 content addressing for sps2
//!
//! This crate provides hashing functionality for content-addressed storage
//! and integrity verification.

mod file_hasher;

pub use file_hasher::{calculate_file_storage_path, FileHashResult, FileHasher, FileHasherConfig};

use blake3::Hasher;
use serde::{Deserialize, Serialize};
use sps2_errors::{Error, StorageError};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Size of chunks for streaming hash computation
const CHUNK_SIZE: usize = 64 * 1024; // 64KB

/// A BLAKE3 hash value
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hash {
    bytes: [u8; 32],
}

impl Hash {
    /// Create a hash from raw bytes
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// Get the raw bytes
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Convert to hex string
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.bytes)
    }

    /// Parse from hex string
    ///
    /// # Errors
    /// Returns an error if the input string is not valid hexadecimal or is not exactly 64 characters (32 bytes).
    pub fn from_hex(s: &str) -> Result<Self, Error> {
        let bytes = hex::decode(s).map_err(|e| StorageError::CorruptedData {
            message: format!("invalid hex: {e}"),
        })?;

        if bytes.len() != 32 {
            return Err(StorageError::CorruptedData {
                message: format!("hash must be 32 bytes, got {}", bytes.len()),
            }
            .into());
        }

        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(Self::from_bytes(array))
    }

    /// Compute hash of a byte slice
    #[must_use]
    pub fn from_data(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        Self::from_bytes(*hash.as_bytes())
    }

    /// Compute hash of a file
    ///
    /// # Errors
    /// Returns an error if the file cannot be opened, read, or if any I/O operation fails.
    pub async fn hash_file(path: &Path) -> Result<Self, Error> {
        let mut file = File::open(path)
            .await
            .map_err(|_| StorageError::PathNotFound {
                path: path.display().to_string(),
            })?;

        let mut hasher = Hasher::new();
        let mut buffer = vec![0; CHUNK_SIZE];

        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }

        Ok(Self::from_bytes(*hasher.finalize().as_bytes()))
    }

    /// Compute hash while copying data to a writer
    ///
    /// # Errors
    /// Returns an error if reading from the reader or writing to the writer fails.
    pub async fn hash_and_copy<R, W>(mut reader: R, mut writer: W) -> Result<(Self, u64), Error>
    where
        R: AsyncReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        let mut hasher = Hasher::new();
        let mut buffer = vec![0; CHUNK_SIZE];
        let mut total_bytes = 0u64;

        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 {
                break;
            }

            hasher.update(&buffer[..n]);
            writer.write_all(&buffer[..n]).await?;
            total_bytes += n as u64;
        }

        writer.flush().await?;
        Ok((Self::from_bytes(*hasher.finalize().as_bytes()), total_bytes))
    }

    /// Compute deterministic hash of a directory's contents
    ///
    /// This creates a reproducible hash by:
    /// 1. Sorting all files by relative path
    /// 2. Hashing each file's relative path, permissions, and contents
    /// 3. Combining all hashes in a deterministic order
    ///
    /// # Errors
    /// Returns an error if directory traversal or file operations fail.
    pub async fn hash_directory(dir_path: &Path) -> Result<Self, Error> {
        // Collect all files with their metadata
        let mut files = BTreeMap::new();
        collect_files(dir_path, dir_path, &mut files).await?;

        // Create a hasher for the entire directory
        let mut dir_hasher = Hasher::new();

        // Process files in sorted order
        for (rel_path, (full_path, metadata)) in files {
            // Hash the relative path
            dir_hasher.update(rel_path.as_bytes());
            dir_hasher.update(b"\0"); // null separator

            // Hash file type and permissions
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode();
                dir_hasher.update(&mode.to_le_bytes());
            }

            if metadata.is_file() {
                // Hash file contents
                let file_hash = Self::hash_file(&full_path).await?;
                dir_hasher.update(file_hash.as_bytes());
            } else if metadata.is_symlink() {
                // Hash symlink target
                let target = tokio::fs::read_link(&full_path).await?;
                dir_hasher.update(target.to_string_lossy().as_bytes());
            }

            // Add another separator
            dir_hasher.update(b"\0");
        }

        Ok(Self::from_bytes(*dir_hasher.finalize().as_bytes()))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for Hash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

/// Verify a file matches an expected hash
///
/// # Errors
/// Returns an error if the file cannot be read or hashed.
pub async fn verify_file(path: &Path, expected: &Hash) -> Result<bool, Error> {
    let actual = Hash::hash_file(path).await?;
    Ok(actual == *expected)
}

/// Create a content-addressed path from a hash
#[must_use]
pub fn content_path(hash: &Hash) -> String {
    hash.to_hex()
}

/// Helper function to collect all files in a directory recursively
async fn collect_files(
    base_path: &Path,
    current_path: &Path,
    files: &mut BTreeMap<String, (std::path::PathBuf, std::fs::Metadata)>,
) -> Result<(), Error> {
    let mut entries = tokio::fs::read_dir(current_path).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = entry.metadata().await?;

        // Get relative path from base
        let rel_path = path
            .strip_prefix(base_path)
            .map_err(|_| StorageError::IoError {
                message: "failed to compute relative path".to_string(),
            })?
            .to_string_lossy()
            .to_string();

        files.insert(rel_path.clone(), (path.clone(), metadata.clone()));

        // Recurse into directories
        if metadata.is_dir() {
            Box::pin(collect_files(base_path, &path, files)).await?;
        }
    }

    Ok(())
}

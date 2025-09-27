#![warn(mismatched_lifetime_syntaxes)]
#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Dual hashing for sps2: BLAKE3 for downloads, xxHash for local verification
//!
//! This crate provides hashing functionality for content-addressed storage
//! and integrity verification using different algorithms for different purposes.

mod file_hasher;

pub use file_hasher::{calculate_file_storage_path, FileHashResult, FileHasher, FileHasherConfig};

use blake3::Hasher as Blake3Hasher;
use serde::{Deserialize, Serialize};
use sps2_errors::{Error, StorageError};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xxhash_rust::xxh3::Xxh3;

/// Size of chunks for streaming hash computation
const CHUNK_SIZE: usize = 64 * 1024; // 64KB

/// Hash algorithm type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HashAlgorithm {
    /// BLAKE3 - used for download verification
    Blake3,
    /// xxHash 128-bit - used for local verification
    XxHash128,
}

impl Default for HashAlgorithm {
    fn default() -> Self {
        Self::XxHash128 // Default to xxHash for local operations
    }
}

/// A hash value that can use different algorithms
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hash {
    algorithm: HashAlgorithm,
    bytes: Vec<u8>, // Variable length to support different hash sizes
}

impl Hash {
    /// Create a BLAKE3 hash from raw bytes (32 bytes)
    #[must_use]
    pub fn from_blake3_bytes(bytes: [u8; 32]) -> Self {
        Self {
            algorithm: HashAlgorithm::Blake3,
            bytes: bytes.to_vec(),
        }
    }

    /// Create an xxHash 128-bit hash from raw bytes (16 bytes)
    #[must_use]
    pub fn from_xxhash128_bytes(bytes: [u8; 16]) -> Self {
        Self {
            algorithm: HashAlgorithm::XxHash128,
            bytes: bytes.to_vec(),
        }
    }

    /// Get the hash algorithm
    #[must_use]
    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    /// Get the raw bytes
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Convert to hex string
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(&self.bytes)
    }

    /// Parse from hex string (detects algorithm based on length)
    ///
    /// # Errors
    /// Returns an error if the input string is not valid hexadecimal.
    pub fn from_hex(s: &str) -> Result<Self, Error> {
        let bytes = hex::decode(s).map_err(|e| StorageError::CorruptedData {
            message: format!("invalid hex: {e}"),
        })?;

        // Determine algorithm based on length
        match bytes.len() {
            32 => {
                let mut array = [0u8; 32];
                array.copy_from_slice(&bytes);
                Ok(Self::from_blake3_bytes(array))
            }
            16 => {
                let mut array = [0u8; 16];
                array.copy_from_slice(&bytes);
                Ok(Self::from_xxhash128_bytes(array))
            }
            _ => Err(StorageError::CorruptedData {
                message: format!("hash must be 16 or 32 bytes, got {}", bytes.len()),
            }
            .into()),
        }
    }

    /// Compute hash of a byte slice using default algorithm (xxHash128)
    #[must_use]
    pub fn from_data(data: &[u8]) -> Self {
        Self::from_data_with_algorithm(data, HashAlgorithm::default())
    }

    /// Compute hash of a byte slice using specified algorithm
    #[must_use]
    pub fn from_data_with_algorithm(data: &[u8], algorithm: HashAlgorithm) -> Self {
        match algorithm {
            HashAlgorithm::Blake3 => {
                let hash = blake3::hash(data);
                Self::from_blake3_bytes(*hash.as_bytes())
            }
            HashAlgorithm::XxHash128 => {
                let hash = xxhash_rust::xxh3::xxh3_128(data);
                Self::from_xxhash128_bytes(hash.to_le_bytes())
            }
        }
    }

    /// Compute BLAKE3 hash of a byte slice (for download verification)
    #[must_use]
    pub fn blake3_from_data(data: &[u8]) -> Self {
        Self::from_data_with_algorithm(data, HashAlgorithm::Blake3)
    }

    /// Compute xxHash128 hash of a byte slice (for local verification)
    #[must_use]
    pub fn xxhash128_from_data(data: &[u8]) -> Self {
        Self::from_data_with_algorithm(data, HashAlgorithm::XxHash128)
    }

    /// Check if this is a BLAKE3 hash
    #[must_use]
    pub fn is_blake3(&self) -> bool {
        matches!(self.algorithm, HashAlgorithm::Blake3)
    }

    /// Check if this is an xxHash128 hash
    #[must_use]
    pub fn is_xxhash128(&self) -> bool {
        matches!(self.algorithm, HashAlgorithm::XxHash128)
    }

    /// Get the expected byte length for this hash algorithm
    #[must_use]
    pub fn expected_length(&self) -> usize {
        match self.algorithm {
            HashAlgorithm::Blake3 => 32,
            HashAlgorithm::XxHash128 => 16,
        }
    }

    /// Compute hash of a file using default algorithm (xxHash128)
    ///
    /// # Errors
    /// Returns an error if the file cannot be opened, read, or if any I/O operation fails.
    pub async fn hash_file(path: &Path) -> Result<Self, Error> {
        Self::hash_file_with_algorithm(path, HashAlgorithm::default()).await
    }

    /// Compute hash of a file using specified algorithm
    ///
    /// # Errors
    /// Returns an error if the file cannot be opened, read, or if any I/O operation fails.
    pub async fn hash_file_with_algorithm(
        path: &Path,
        algorithm: HashAlgorithm,
    ) -> Result<Self, Error> {
        let mut file = File::open(path)
            .await
            .map_err(|_| StorageError::PathNotFound {
                path: path.display().to_string(),
            })?;

        match algorithm {
            HashAlgorithm::Blake3 => {
                let mut hasher = Blake3Hasher::new();
                let mut buffer = vec![0; CHUNK_SIZE];

                loop {
                    let n = file.read(&mut buffer).await?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buffer[..n]);
                }

                Ok(Self::from_blake3_bytes(*hasher.finalize().as_bytes()))
            }
            HashAlgorithm::XxHash128 => {
                let mut hasher = Xxh3::new();
                let mut buffer = vec![0; CHUNK_SIZE];

                loop {
                    let n = file.read(&mut buffer).await?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buffer[..n]);
                }

                let hash_result = hasher.digest128();
                Ok(Self::from_xxhash128_bytes(hash_result.to_le_bytes()))
            }
        }
    }

    /// Compute BLAKE3 hash of a file (for download verification)
    ///
    /// # Errors
    /// Returns an error if the file cannot be opened, read, or if any I/O operation fails.
    pub async fn blake3_hash_file(path: &Path) -> Result<Self, Error> {
        Self::hash_file_with_algorithm(path, HashAlgorithm::Blake3).await
    }

    /// Compute hash while copying data to a writer using default algorithm (xxHash128)
    ///
    /// # Errors
    /// Returns an error if reading from the reader or writing to the writer fails.
    pub async fn hash_and_copy<R, W>(reader: R, writer: W) -> Result<(Self, u64), Error>
    where
        R: AsyncReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        Self::hash_and_copy_with_algorithm(reader, writer, HashAlgorithm::default()).await
    }

    /// Compute hash while copying data to a writer using specified algorithm
    ///
    /// # Errors
    /// Returns an error if reading from the reader or writing to the writer fails.
    pub async fn hash_and_copy_with_algorithm<R, W>(
        mut reader: R,
        mut writer: W,
        algorithm: HashAlgorithm,
    ) -> Result<(Self, u64), Error>
    where
        R: AsyncReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        let mut buffer = vec![0; CHUNK_SIZE];
        let mut total_bytes = 0u64;

        match algorithm {
            HashAlgorithm::Blake3 => {
                let mut hasher = Blake3Hasher::new();

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
                Ok((
                    Self::from_blake3_bytes(*hasher.finalize().as_bytes()),
                    total_bytes,
                ))
            }
            HashAlgorithm::XxHash128 => {
                let mut hasher = Xxh3::new();

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
                let hash_result = hasher.digest128();
                Ok((
                    Self::from_xxhash128_bytes(hash_result.to_le_bytes()),
                    total_bytes,
                ))
            }
        }
    }

    /// Compute deterministic hash of a directory's contents using default algorithm (xxHash128)
    ///
    /// This creates a reproducible hash by:
    /// 1. Sorting all files by relative path
    /// 2. Hashing each file's relative path, permissions, and contents
    /// 3. Combining all hashes in a deterministic order
    ///
    /// # Errors
    /// Returns an error if directory traversal or file operations fail.
    pub async fn hash_directory(dir_path: &Path) -> Result<Self, Error> {
        Self::hash_directory_with_algorithm(dir_path, HashAlgorithm::default()).await
    }

    /// Compute deterministic hash of a directory's contents using specified algorithm
    ///
    /// # Errors
    /// Returns an error if directory traversal or file operations fail.
    pub async fn hash_directory_with_algorithm(
        dir_path: &Path,
        algorithm: HashAlgorithm,
    ) -> Result<Self, Error> {
        // Collect all files with their metadata
        let mut files = BTreeMap::new();
        collect_files(dir_path, dir_path, &mut files).await?;

        match algorithm {
            HashAlgorithm::Blake3 => {
                // Create a hasher for the entire directory
                let mut dir_hasher = Blake3Hasher::new();

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
                        let file_hash =
                            Self::hash_file_with_algorithm(&full_path, algorithm).await?;
                        dir_hasher.update(file_hash.as_bytes());
                    } else if metadata.is_symlink() {
                        // Hash symlink target
                        let target = tokio::fs::read_link(&full_path).await?;
                        dir_hasher.update(target.to_string_lossy().as_bytes());
                    }

                    // Add another separator
                    dir_hasher.update(b"\0");
                }

                Ok(Self::from_blake3_bytes(*dir_hasher.finalize().as_bytes()))
            }
            HashAlgorithm::XxHash128 => {
                // Create a hasher for the entire directory
                let mut dir_hasher = Xxh3::new();

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
                        let file_hash =
                            Self::hash_file_with_algorithm(&full_path, algorithm).await?;
                        dir_hasher.update(file_hash.as_bytes());
                    } else if metadata.is_symlink() {
                        // Hash symlink target
                        let target = tokio::fs::read_link(&full_path).await?;
                        dir_hasher.update(target.to_string_lossy().as_bytes());
                    }

                    // Add another separator
                    dir_hasher.update(b"\0");
                }

                let hash_result = dir_hasher.digest128();
                Ok(Self::from_xxhash128_bytes(hash_result.to_le_bytes()))
            }
        }
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

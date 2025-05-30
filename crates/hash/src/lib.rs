#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! BLAKE3 content addressing for spsv2
//!
//! This crate provides hashing functionality for content-addressed storage
//! and integrity verification.

use blake3::Hasher;
use serde::{Deserialize, Serialize};
use spsv2_errors::{Error, StorageError};
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
    let hex = hash.to_hex();
    // Use first 2 chars as directory for better filesystem performance
    format!("{}/{}", &hex[..2], &hex[2..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    // AsyncWriteExt removed - not used in tests

    #[test]
    fn test_hash_basics() {
        let data = b"hello world";
        let hash = Hash::from_data(data);

        // Known BLAKE3 hash of "hello world"
        let expected = "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24";
        assert_eq!(hash.to_hex(), expected);
    }

    #[test]
    fn test_hash_serialization() {
        let hash = Hash::from_data(b"test");
        let json = serde_json::to_string(&hash).unwrap();
        let deserialized: Hash = serde_json::from_str(&json).unwrap();
        assert_eq!(hash, deserialized);
    }

    #[tokio::test]
    async fn test_hash_file() {
        use std::io::Write;
        let mut temp = NamedTempFile::new().unwrap();
        let data = b"test file content";
        temp.write_all(data).unwrap();

        let hash = Hash::hash_file(temp.path()).await.unwrap();
        let expected = Hash::from_data(data);
        assert_eq!(hash, expected);
    }

    #[tokio::test]
    async fn test_hash_and_copy() {
        let data = b"data to copy";
        let reader = std::io::Cursor::new(data);
        let mut writer = Vec::new();

        let (hash, bytes) = Hash::hash_and_copy(reader, &mut writer).await.unwrap();

        assert_eq!(writer, data);
        assert_eq!(bytes, data.len() as u64);
        assert_eq!(hash, Hash::from_data(data));
    }

    #[test]
    fn test_content_path() {
        let hash = Hash::from_data(b"test");
        let path = content_path(&hash);
        assert!(path.starts_with("48/"));
    }
}

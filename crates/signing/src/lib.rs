#![deny(clippy::pedantic, unsafe_code)]

use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sps2_errors::{Error, OpsError};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    Minisign,
    // OpenPgp (future)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicKeyRef {
    pub id: String,
    pub algo: Algorithm,
    pub data: String,
}

/// Verify content at `content_path` against a minisign signature string using any of the provided trusted keys.
/// Returns the key id that successfully verified.
///
/// # Errors
/// Returns an error if the content cannot be read, the signature cannot be parsed,
/// or if verification fails for all provided keys.
pub fn verify_minisign_file_with_keys(
    content_path: &Path,
    signature_str: &str,
    trusted_keys: &[PublicKeyRef],
) -> Result<String, Error> {
    let content = fs::read(content_path).map_err(|e| {
        Error::internal(format!(
            "Failed to read content for signature verification: {e}"
        ))
    })?;
    verify_minisign_bytes_with_keys(&content, signature_str, trusted_keys)
}

/// Verify raw bytes against a minisign signature string using any of the provided trusted keys.
/// Returns the key id that successfully verified.
///
/// # Errors
/// Returns an error if the signature cannot be parsed or if no trusted key verifies the content.
pub fn verify_minisign_bytes_with_keys(
    content: &[u8],
    signature_str: &str,
    trusted_keys: &[PublicKeyRef],
) -> Result<String, Error> {
    if trusted_keys.is_empty() {
        return Err(OpsError::RepoSyncFailed {
            message: "No trusted keys available for verification".to_string(),
        }
        .into());
    }

    // Parse signature (full minisign string including comment line)
    let sig = Signature::decode(signature_str).map_err(|e| OpsError::RepoSyncFailed {
        message: format!("Invalid signature format: {e}"),
    })?;

    let mut last_err = None;
    for key in trusted_keys {
        if key.algo != Algorithm::Minisign {
            continue;
        }
        match PublicKey::from_base64(&key.data) {
            Ok(pk) => match pk.verify(content, &sig, false) {
                Ok(()) => return Ok(key.id.clone()),
                Err(e) => {
                    last_err = Some(format!("{e}"));
                }
            },
            Err(e) => {
                last_err = Some(format!("Invalid trusted key format for {}: {e}", key.id));
            }
        }
    }

    Err(OpsError::RepoSyncFailed {
        message: format!(
            "Signature verification failed with {} trusted keys. Last error: {}",
            trusted_keys.len(),
            last_err.unwrap_or_else(|| "unknown".to_string())
        ),
    }
    .into())
}

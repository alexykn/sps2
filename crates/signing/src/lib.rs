#![deny(clippy::pedantic, unsafe_code)]

use base64::{engine::general_purpose, Engine as _};
use minisign::{sign, SecretKeyBox};
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sps2_errors::{Error, SigningError};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    Minisign,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicKeyRef {
    pub id: String,
    pub algo: Algorithm,
    pub data: String,
}

/// A hack to extract the key ID from a raw signature string because the `minisign_verify`
/// crate doesn't expose it publicly.
fn extract_key_id_from_sig_str(signature_str: &str) -> Result<String, SigningError> {
    let mut lines = signature_str.lines();
    lines.next(); // Skip untrusted comment
    let sig_line = lines.next().ok_or_else(|| {
        SigningError::InvalidSignatureFormat("Missing signature line".to_string())
    })?;
    let decoded_sig = general_purpose::STANDARD.decode(sig_line).map_err(|e| {
        SigningError::InvalidSignatureFormat(format!("Failed to decode signature line: {e}"))
    })?;
    if decoded_sig.len() < 10 {
        return Err(SigningError::InvalidSignatureFormat(
            "Signature line is too short".to_string(),
        ));
    }
    let key_id_bytes = &decoded_sig[2..10];
    Ok(hex::encode(key_id_bytes))
}

/// Verify content at `content_path` against a minisign signature string using any of the provided trusted keys.
///
/// # Errors
///
/// Returns an error if:
/// - The content file cannot be read
/// - The signature verification fails
/// - No matching trusted key is found
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
    Ok(verify_minisign_bytes_with_keys(
        &content,
        signature_str,
        trusted_keys,
    )?)
}

/// Verify raw bytes against a minisign signature string using any of the provided trusted keys.
///
/// # Errors
///
/// Returns an error if:
/// - The signature string cannot be decoded
/// - The public key cannot be decoded from base64
/// - The signature verification fails
/// - No matching trusted key is found
pub fn verify_minisign_bytes_with_keys(
    content: &[u8],
    signature_str: &str,
    trusted_keys: &[PublicKeyRef],
) -> Result<String, SigningError> {
    let key_id_from_sig = extract_key_id_from_sig_str(signature_str)?;

    let sig = Signature::decode(signature_str)
        .map_err(|e| SigningError::InvalidSignatureFormat(e.to_string()))?;

    for key in trusted_keys {
        if key.algo != Algorithm::Minisign {
            continue;
        }

        if key.id == key_id_from_sig {
            let pk = PublicKey::from_base64(&key.data)
                .map_err(|e| SigningError::InvalidPublicKey(e.to_string()))?;

            return match pk.verify(content, &sig, false) {
                Ok(()) => Ok(key.id.clone()),
                Err(e) => Err(SigningError::VerificationFailed {
                    reason: e.to_string(),
                }),
            };
        }
    }

    Err(SigningError::NoTrustedKeyFound {
        key_id: key_id_from_sig,
    })
}

/// Sign raw bytes with a Minisign secret key file and return the signature string.
///
/// The secret key file is expected to be in Minisign "secret key box" format.
/// If the key is encrypted, provide the `passphrase_or_keychain` string as required by
/// the underlying minisign crate (for macOS keychain integration or passphrase).
///
/// # Errors
///
/// Returns an error if the key cannot be read/parsed, the secret key cannot be decrypted,
/// or the signing operation fails.
pub fn minisign_sign_bytes(
    bytes: &[u8],
    secret_key_path: &std::path::Path,
    passphrase_or_keychain: Option<&str>,
    trusted_comment: Option<&str>,
    untrusted_comment: Option<&str>,
) -> Result<String, Error> {
    use std::io::Cursor;
    let sk_box_str = std::fs::read_to_string(secret_key_path)
        .map_err(|e| Error::internal(format!("Failed to read secret key file: {e}")))?;

    let sk_box = SecretKeyBox::from_string(&sk_box_str)
        .map_err(|e| Error::internal(format!("Failed to parse secret key: {e}")))?;

    let secret_key = sk_box
        .into_secret_key(passphrase_or_keychain.map(std::string::ToString::to_string))
        .map_err(|e| Error::internal(format!("Failed to decrypt secret key: {e}")))?;

    let signature = sign(
        None,
        &secret_key,
        Cursor::new(bytes),
        trusted_comment,
        untrusted_comment,
    )
    .map_err(|e| Error::internal(format!("Failed to sign bytes: {e}")))?;

    Ok(signature.into_string())
}

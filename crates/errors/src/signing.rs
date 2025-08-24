#!/deny(clippy::pedantic, unsafe_code)

//! Signing error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SigningError {
    #[error("signature verification failed: {reason}")]
    VerificationFailed { reason: String },

    #[error("no trusted key found for signature with key id: {key_id}")]
    NoTrustedKeyFound { key_id: String },

    #[error("invalid signature format: {0}")]
    InvalidSignatureFormat(String),

    #[error("invalid public key format: {0}")]
    InvalidPublicKey(String),
}

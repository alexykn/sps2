#![deny(clippy::pedantic, unsafe_code)]

//! Signing error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
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

impl UserFacingError for SigningError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::VerificationFailed { .. } => Some("Ensure you have the correct public key and the artifact has not been tampered with."),
            Self::NoTrustedKeyFound { .. } => Some("Import the missing public key (`sps2 keys import`) and retry."),
            Self::InvalidSignatureFormat { .. } | Self::InvalidPublicKey { .. } => {
                Some("Check the signature and key files for corruption or unsupported formats.")
            }
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(self, Self::NoTrustedKeyFound { .. })
    }
}

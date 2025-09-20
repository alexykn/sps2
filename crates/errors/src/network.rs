//! Network-related error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

const HINT_CHECK_CONNECTION: &str = "Check your network connection and retry.";
const HINT_RETRY_LATER: &str = "Retry the operation; the service may recover shortly.";

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum NetworkError {
    #[error("connection timeout to {url}")]
    Timeout { url: String },

    #[error("download failed: {0}")]
    DownloadFailed(String),

    #[error("connection refused: {0}")]
    ConnectionRefused(String),

    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    #[error("HTTP error {status}: {message}")]
    HttpError { status: u16, message: String },

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("SSL/TLS error: {0}")]
    TlsError(String),

    #[error("network unavailable")]
    NetworkUnavailable,

    #[error("rate limited: retry after {seconds} seconds")]
    RateLimited { seconds: u64 },

    #[error("partial content not supported for resumable download")]
    PartialContentNotSupported,

    #[error("content length mismatch: expected {expected}, got {actual}")]
    ContentLengthMismatch { expected: u64, actual: u64 },

    #[error("range request failed: {message}")]
    RangeRequestFailed { message: String },

    #[error("file size exceeds limit: {size} bytes > {limit} bytes")]
    FileSizeExceeded { size: u64, limit: u64 },

    #[error("stream interrupted after {bytes} bytes")]
    StreamInterrupted { bytes: u64 },

    #[error("unsupported protocol: {protocol}")]
    UnsupportedProtocol { protocol: String },
}

impl UserFacingError for NetworkError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::Timeout { .. } | Self::NetworkUnavailable => Some(HINT_CHECK_CONNECTION),
            Self::RateLimited { .. } => Some("Wait for the rate limit window to expire."),
            Self::PartialContentNotSupported | Self::RangeRequestFailed { .. } => {
                Some("Retry without resume or select a different mirror.")
            }
            Self::StreamInterrupted { .. } => Some(HINT_RETRY_LATER),
            Self::ChecksumMismatch { .. } => {
                Some("Retry with `--no-cache` or verify the artifact.")
            }
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout { .. }
                | Self::DownloadFailed(_)
                | Self::ConnectionRefused(_)
                | Self::NetworkUnavailable
                | Self::RateLimited { .. }
                | Self::PartialContentNotSupported
                | Self::ContentLengthMismatch { .. }
                | Self::StreamInterrupted { .. }
        )
    }
}

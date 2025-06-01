//! Network-related error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

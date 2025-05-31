#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Network operations for sps2
//!
//! This crate handles all HTTP operations including package downloads,
//! index fetching, and connection pooling with retry logic.

mod client;
mod download;

pub use client::{DownloadProgress, NetClient, NetConfig};
pub use download::{Download, DownloadResult};

use sps2_errors::{Error, NetworkError};
use sps2_events::{Event, EventSender, EventSenderExt};
use sps2_hash::Hash;
use std::path::Path;
use url::Url;

/// Download a file with progress reporting
///
/// # Errors
///
/// Returns an error if the URL is invalid, the download fails, or there are
/// I/O errors while writing the file.
pub async fn download_file(
    client: &NetClient,
    url: &str,
    dest: &Path,
    expected_hash: Option<&Hash>,
    tx: &EventSender,
) -> Result<DownloadResult, Error> {
    let download = Download::new(url)?;
    download.execute(client, dest, expected_hash, tx).await
}

/// Fetch text content from a URL
///
/// # Errors
///
/// Returns an error if the HTTP request fails, the server returns an error status,
/// or the response body cannot be decoded as text.
pub async fn fetch_text(client: &NetClient, url: &str, tx: &EventSender) -> Result<String, Error> {
    tx.emit(Event::DebugLog {
        message: format!("Fetching text from {url}"),
        context: std::collections::HashMap::new(),
    });

    let response = client.get(url).await?;

    if !response.status().is_success() {
        return Err(NetworkError::HttpError {
            status: response.status().as_u16(),
            message: response.status().to_string(),
        }
        .into());
    }

    response
        .text()
        .await
        .map_err(|e| NetworkError::DownloadFailed(e.to_string()).into())
}

/// Fetch binary content from a URL
///
/// # Errors
///
/// Returns an error if the HTTP request fails, the server returns an error status,
/// or the response body cannot be read as bytes.
pub async fn fetch_bytes(
    client: &NetClient,
    url: &str,
    tx: &EventSender,
) -> Result<Vec<u8>, Error> {
    tx.emit(Event::DebugLog {
        message: format!("Fetching bytes from {url}"),
        context: std::collections::HashMap::new(),
    });

    let response = client.get(url).await?;

    if !response.status().is_success() {
        return Err(NetworkError::HttpError {
            status: response.status().as_u16(),
            message: response.status().to_string(),
        }
        .into());
    }

    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| NetworkError::DownloadFailed(e.to_string()).into())
}

/// Check if a URL is accessible
///
/// # Errors
///
/// Returns an error if there are network issues preventing the HEAD request.
/// Note: This function returns `Ok(false)` for inaccessible URLs rather than errors.
pub async fn check_url(client: &NetClient, url: &str) -> Result<bool, Error> {
    match client.head(url).await {
        Ok(response) => Ok(response.status().is_success()),
        Err(_) => Ok(false),
    }
}

/// Parse and validate a URL
///
/// # Errors
///
/// Returns an error if the URL string is malformed or invalid according to RFC 3986.
pub fn parse_url(url: &str) -> Result<Url, Error> {
    Url::parse(url).map_err(|e| NetworkError::InvalidUrl(e.to_string()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url() {
        assert!(parse_url("https://example.com").is_ok());
        assert!(parse_url("not a url").is_err());
    }
}

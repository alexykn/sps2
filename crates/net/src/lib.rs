#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Network operations for sps2
//!
//! This crate handles all HTTP operations including package downloads,
//! index fetching, and connection pooling with retry logic.

mod client;
mod download;

pub use client::{NetClient, NetConfig};
pub use download::{
    DownloadResult, PackageDownloadConfig, PackageDownloadRequest, PackageDownloadResult,
    PackageDownloader, RetryConfig,
};

use sps2_errors::{Error, NetworkError};
use sps2_events::{AppEvent, EventEmitter, EventSender, GeneralEvent};
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
    _client: &NetClient,
    url: &str,
    dest: &Path,
    expected_hash: Option<&Hash>,
    tx: &EventSender,
) -> Result<(Hash, u64), Error> {
    let downloader = PackageDownloader::with_defaults(sps2_events::ProgressManager::new())?;
    let result = downloader
        .download_with_resume(
            url,
            dest,
            expected_hash,
            "simple_download".to_string(),
            None,
            tx.clone(),
        )
        .await?;
    Ok((result.hash, result.size))
}

/// Fetch text content from a URL
///
/// # Errors
///
/// Returns an error if the HTTP request fails, the server returns an error status,
/// or the response body cannot be decoded as text.
pub async fn fetch_text(client: &NetClient, url: &str, tx: &EventSender) -> Result<String, Error> {
    tx.emit(AppEvent::General(GeneralEvent::debug(format!(
        "Fetching text from {url}"
    ))));

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

/// Conditionally fetch text content from a URL with `ETag` support
///
/// # Errors
///
/// Returns an error if the HTTP request fails, the server returns an error status,
/// or the response body cannot be decoded as text.
///
/// # Returns
///
/// Returns `Ok(None)` if the server responds with 304 Not Modified,
/// `Ok(Some((content, etag)))` if new content is available.
pub async fn fetch_text_conditional(
    client: &NetClient,
    url: &str,
    etag: Option<&str>,
    tx: &EventSender,
) -> Result<Option<(String, Option<String>)>, Error> {
    tx.emit(AppEvent::General(GeneralEvent::debug(format!(
        "Fetching text from {url} with conditional request"
    ))));

    let mut headers = Vec::new();
    if let Some(etag_value) = etag {
        headers.push(("If-None-Match", etag_value));
    }

    let response = client.get_with_headers(url, &headers).await?;

    // Handle 304 Not Modified
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        tx.emit(AppEvent::General(GeneralEvent::debug(
            "Server returned 304 Not Modified - using cached content",
        )));
        return Ok(None);
    }

    if !response.status().is_success() {
        return Err(NetworkError::HttpError {
            status: response.status().as_u16(),
            message: response.status().to_string(),
        }
        .into());
    }

    // Extract new ETag from response headers
    let new_etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let content = response
        .text()
        .await
        .map_err(|e| NetworkError::DownloadFailed(e.to_string()))?;

    Ok(Some((content, new_etag)))
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
    tx.emit(AppEvent::General(GeneralEvent::debug(format!(
        "Fetching bytes from {url}"
    ))));

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

/// Fetch and deserialize JSON content from a URL
///
/// # Errors
///
/// Returns an error if the HTTP request fails, the server returns an error status,
/// or the response body cannot be deserialized from JSON.
pub async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &NetClient,
    url: &str,
    tx: &EventSender,
) -> Result<T, Error> {
    let text = fetch_text(client, url, tx).await?;
    serde_json::from_str(&text).map_err(|e| {
        sps2_errors::OpsError::SerializationError {
            message: e.to_string(),
        }
        .into()
    })
}

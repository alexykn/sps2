//! URL validation and HTTP response validation for downloads

use sps2_errors::{Error, NetworkError};
use url::Url;

/// Validate URL and check for supported protocols
pub(super) fn validate_url(url: &str) -> Result<String, Error> {
    let parsed = Url::parse(url).map_err(|e| NetworkError::InvalidUrl(e.to_string()))?;

    match parsed.scheme() {
        "http" | "https" | "file" => Ok(url.to_string()),
        scheme => Err(NetworkError::UnsupportedProtocol {
            protocol: scheme.to_string(),
        }
        .into()),
    }
}

/// Validate HTTP response for download
pub(super) fn validate_response(
    response: &reqwest::Response,
    is_resume: bool,
) -> Result<(), Error> {
    let status = response.status();

    if is_resume {
        if status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(NetworkError::PartialContentNotSupported.into());
        }
    } else if !status.is_success() {
        return Err(NetworkError::HttpError {
            status: status.as_u16(),
            message: status.to_string(),
        }
        .into());
    }

    Ok(())
}

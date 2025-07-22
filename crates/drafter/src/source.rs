//! Source location handling and preparation

use crate::Result;
use sps2_errors::BuildError;
use sps2_events::EventEmitter;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Represents different source locations for drafting
#[derive(Debug, Clone)]
pub enum SourceLocation {
    /// Local directory path
    Local(PathBuf),
    /// Git repository URL
    Git(String),
    /// Remote URL to download
    Url(String),
    /// Local archive file
    Archive(PathBuf),
}

impl SourceLocation {
    /// Get a display string for the source location
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Local(path) | Self::Archive(path) => path.display().to_string(),
            Self::Git(url) | Self::Url(url) => url.clone(),
        }
    }

    /// Check if this is a remote source (needs downloading)
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Git(_) | Self::Url(_))
    }
}

/// Prepare source directory from various source locations
///
/// Returns (optional temp dir to keep alive, actual source directory path)
pub async fn prepare(
    source: &SourceLocation,
    event_tx: Option<&sps2_events::EventSender>,
) -> Result<(Option<TempDir>, PathBuf)> {
    match source {
        SourceLocation::Local(path) => prepare_local(path),
        SourceLocation::Git(url) => prepare_git(url, event_tx).await,
        SourceLocation::Url(url) => prepare_url(url, event_tx).await,
        SourceLocation::Archive(path) => prepare_archive(path, event_tx).await,
    }
}

/// Prepare from local directory
fn prepare_local(path: &Path) -> Result<(Option<TempDir>, PathBuf)> {
    if !path.exists() {
        return Err(BuildError::DraftSourceFailed {
            message: format!("Local path does not exist: {}", path.display()),
        }
        .into());
    }

    if !path.is_dir() {
        return Err(BuildError::DraftSourceFailed {
            message: format!("Local path is not a directory: {}", path.display()),
        }
        .into());
    }

    Ok((None, path.to_path_buf()))
}

/// Prepare from git repository
async fn prepare_git(
    url: &str,
    event_tx: Option<&sps2_events::EventSender>,
) -> Result<(Option<TempDir>, PathBuf)> {
    // Send progress event
    if let Some(tx) = event_tx {
        tx.emit_operation_started(format!("Cloning git repository: {url}"));
    }

    let temp_dir = TempDir::new().map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Failed to create temp directory: {e}"),
    })?;

    // Extract repository name from URL
    let repo_name = url
        .split('/')
        .next_back()
        .and_then(|s| s.strip_suffix(".git").or(Some(s)))
        .ok_or_else(|| BuildError::InvalidUrl {
            url: url.to_string(),
        })?;

    let repo_path = temp_dir.path().join(repo_name);

    // Clone using git command for better compatibility
    let output = tokio::process::Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            url,
            &repo_path.display().to_string(),
        ])
        .current_dir(temp_dir.path())
        .output()
        .await?;

    if !output.status.success() {
        return Err(BuildError::GitCloneFailed {
            message: format!(
                "Failed to clone {url}: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        }
        .into());
    }

    Ok((Some(temp_dir), repo_path))
}

/// Prepare from URL download
async fn prepare_url(
    url: &str,
    event_tx: Option<&sps2_events::EventSender>,
) -> Result<(Option<TempDir>, PathBuf)> {
    // Send progress event
    if let Some(tx) = event_tx {
        tx.emit_operation_started(format!("Downloading source from: {url}"));
    }

    let temp_dir = TempDir::new().map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Failed to create temp directory: {e}"),
    })?;

    // Extract filename from URL
    let filename = url
        .split('/')
        .next_back()
        .ok_or_else(|| BuildError::InvalidUrl {
            url: url.to_string(),
        })?;

    let download_path = temp_dir.path().join(filename);

    // Download the file using sps2-net
    let client = sps2_net::NetClient::new(sps2_net::NetConfig::default())?;
    let response = client.get(url).await?;
    let bytes = response.bytes().await.map_err(|e| {
        sps2_errors::NetworkError::DownloadFailed(format!("Failed to download {url}: {e}"))
    })?;
    tokio::fs::write(&download_path, &bytes).await?;

    // No hash calculation needed - files are downloaded without validation

    // Extract the archive
    let extract_dir = temp_dir.path().join("extracted");
    tokio::fs::create_dir(&extract_dir).await?;
    crate::archive::extract(&download_path, &extract_dir).await?;

    // Find the actual source directory (might be nested)
    let source_dir = find_source_root(&extract_dir).await?;

    Ok((Some(temp_dir), source_dir))
}

/// Prepare from local archive
async fn prepare_archive(
    path: &Path,
    event_tx: Option<&sps2_events::EventSender>,
) -> Result<(Option<TempDir>, PathBuf)> {
    if !path.exists() {
        return Err(BuildError::DraftSourceFailed {
            message: format!("Archive file does not exist: {}", path.display()),
        }
        .into());
    }

    // Send progress event
    if let Some(tx) = event_tx {
        tx.emit_operation_started(format!("Extracting archive: {}", path.display()));
    }

    let temp_dir = TempDir::new().map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Failed to create temp directory: {e}"),
    })?;

    // No hash calculation needed - archives are processed without validation

    let extract_dir = temp_dir.path().to_path_buf();
    crate::archive::extract(path, &extract_dir).await?;

    // Find the actual source directory (might be nested)
    let source_dir = find_source_root(&extract_dir).await?;

    Ok((Some(temp_dir), source_dir))
}

/// Find the actual source root within an extracted directory
/// (handles common case where archive contains a single top-level directory)
async fn find_source_root(dir: &Path) -> Result<PathBuf> {
    let mut entries = tokio::fs::read_dir(dir).await?;
    let mut count = 0;
    let mut single_dir = None;

    while let Some(entry) = entries.next_entry().await? {
        count += 1;
        if count > 1 {
            // Multiple entries, use the directory as-is
            return Ok(dir.to_path_buf());
        }
        if entry.file_type().await?.is_dir() {
            single_dir = Some(entry.path());
        }
    }

    // If there's exactly one directory, use that as the source root
    if count == 1 {
        if let Some(dir) = single_dir {
            Ok(dir)
        } else {
            Ok(dir.to_path_buf())
        }
    } else {
        Ok(dir.to_path_buf())
    }
}

//! Utility functions for staging operations
//!
//! This module provides utility functions for file system operations
//! used throughout the staging module.

use sps2_errors::{Error, InstallError};
use sps2_events::{AppEvent, EventEmitter, EventSender, GeneralEvent};
use std::path::Path;
use tokio::fs;

/// Count files in a directory recursively
pub async fn count_files(path: &Path) -> Result<usize, Error> {
    let mut count = 0;
    let mut entries = fs::read_dir(path)
        .await
        .map_err(|e| InstallError::FilesystemError {
            operation: "count_files".to_string(),
            path: path.display().to_string(),
            message: e.to_string(),
        })?;

    while let Some(entry) =
        entries
            .next_entry()
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "count_files_entry".to_string(),
                path: path.display().to_string(),
                message: e.to_string(),
            })?
    {
        count += 1;
        let entry_path = entry.path();

        if entry_path.is_dir() {
            count += Box::pin(count_files(&entry_path)).await?;
        }
    }

    Ok(count)
}

/// Debug helper to list directory contents
pub async fn debug_list_directory_contents(path: &Path, sender: &EventSender) {
    if let Ok(mut entries) = tokio::fs::read_dir(path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: format!("DEBUG: Found file: {}", entry.file_name().to_string_lossy()),
                context: std::collections::HashMap::new(),
            }));
        }
    }
}

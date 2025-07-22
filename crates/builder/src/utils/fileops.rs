//! File system operations for build processes

use sps2_errors::Error;
use std::path::Path;
use tokio::fs;

/// Recursively copy directory contents
pub fn copy_directory_recursive<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>> {
    Box::pin(async move {
        fs::create_dir_all(dst).await?;

        let mut entries = fs::read_dir(src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if entry_path.is_dir() {
                copy_directory_recursive(&entry_path, &dst_path).await?;
            } else {
                fs::copy(&entry_path, &dst_path).await?;
            }
        }

        Ok(())
    })
}

/// Recursively copy directory contents while stripping the opt/pm/live prefix
pub fn copy_directory_strip_live_prefix<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>> {
    Box::pin(async move {
        fs::create_dir_all(dst).await?;

        // Look for opt/pm/live subdirectory in the staging directory
        let live_prefix_path = src.join("opt").join("pm").join("live");

        if live_prefix_path.exists() && live_prefix_path.is_dir() {
            // Copy contents of opt/pm/live directly to dst, stripping the prefix
            copy_directory_recursive(&live_prefix_path, dst).await?;
        } else {
            // Fallback: copy everything as-is if no opt/pm/live structure found
            copy_directory_recursive(src, dst).await?;
        }

        Ok(())
    })
}

/// Copy source files from recipe directory to working directory (excluding .star files)
pub async fn copy_source_files(
    recipe_dir: &Path,
    working_dir: &Path,
    context: &crate::BuildContext,
) -> Result<(), Error> {
    use crate::utils::events::send_event;
    use sps2_events::{AppEvent, GeneralEvent};

    send_event(
        context,
AppEvent::General(GeneralEvent::debug(
            "Cleaning up temporary files"
        )),
    );

    let mut entries = fs::read_dir(recipe_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let entry_path = entry.path();
        let file_name = entry.file_name();
        let dest_path = working_dir.join(&file_name);

        if entry_path.is_dir() {
            // Recursively copy directories
            copy_directory_recursive(&entry_path, &dest_path).await?;

            send_event(
                context,
                AppEvent::General(GeneralEvent::debug(
                    &format!(
                        "Copied directory {} to {}",
                        file_name.to_string_lossy(),
                        dest_path.display()
                    ),
                )),
            );
        } else if entry_path.extension().is_none_or(|ext| ext != "star") {
            // Copy files except .star files
            fs::copy(&entry_path, &dest_path).await?;

            send_event(
                context,
                AppEvent::General(GeneralEvent::debug(
                    &format!(
                        "Copied {} to {}",
                        file_name.to_string_lossy(),
                        dest_path.display()
                    ),
                )),
            );
        }
    }

    Ok(())
}

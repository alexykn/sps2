//! Draft operation for generating build recipes

use crate::OpsCtx;
use sps2_drafter::{Drafter, SourceLocation};
use sps2_errors::{Error, OpsError};
use std::path::PathBuf;

/// Draft a recipe from a source location
///
/// # Errors
///
/// Returns an error if:
/// - The source location is invalid
/// - The drafter fails to generate a recipe
/// - The output file cannot be written
pub async fn draft_recipe(
    ctx: &OpsCtx,
    path: Option<PathBuf>,
    git: Option<String>,
    url: Option<String>,
    archive: Option<PathBuf>,
    output: Option<PathBuf>,
) -> Result<(), Error> {
    // Convert CLI source to drafter SourceLocation
    let source_location = if let Some(path) = path {
        SourceLocation::Local(path)
    } else if let Some(git) = git {
        SourceLocation::Git(git)
    } else if let Some(url) = url {
        SourceLocation::Url(url)
    } else if let Some(archive) = archive {
        SourceLocation::Archive(archive)
    } else {
        return Err(OpsError::InvalidOperation {
            operation: "No source specified for draft command".to_string(),
        }
        .into());
    };

    // Create drafter with event sender
    let drafter = Drafter::new(source_location).with_event_sender(ctx.tx.clone());

    // Run the drafter
    let draft_result = drafter.run().await.map_err(|e| OpsError::OperationFailed {
        message: format!("Failed to draft recipe: {e}"),
    })?;

    // Determine output path
    let output_path = if let Some(path) = output {
        if path.is_dir() {
            path.join(format!(
                "{}-{}.yaml",
                draft_result.metadata.name, draft_result.metadata.version
            ))
        } else {
            path
        }
    } else {
        let filename = format!(
            "{}-{}.yaml",
            draft_result.metadata.name, draft_result.metadata.version
        );
        std::env::current_dir()
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to get current directory: {e}"),
            })?
            .join(filename)
    };

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to create output directory: {e}"),
            })?;
    }

    // Write the recipe
    tokio::fs::write(&output_path, &draft_result.recipe_content)
        .await
        .map_err(|e| OpsError::OperationFailed {
            message: format!("Failed to write recipe file: {e}"),
        })?;

    // Send success event
    let _ = ctx.tx.send(sps2_events::Event::OperationCompleted {
        operation: format!(
            "YAML recipe successfully written to {}",
            output_path.display()
        ),
        success: true,
    });

    Ok(())
}

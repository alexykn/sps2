//! Cleaner that removes object (.o) files

use crate::artifact_qa::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::Event;

pub struct ObjectFileCleaner;

impl crate::artifact_qa::traits::Action for ObjectFileCleaner {
    const NAME: &'static str = "Object file cleaner";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&crate::artifact_qa::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
        let staging_dir = env.staging_dir();
        let mut removed_files = Vec::new();

        // Walk staging directory for .o files
        for entry in ignore::WalkBuilder::new(staging_dir)
            .hidden(false)
            .parents(false)
            .build()
        {
            let path = match entry {
                Ok(e) => e.into_path(),
                Err(_) => continue,
            };

            if !path.is_file() {
                continue;
            }

            // Check if it's a .o file
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "o" {
                    // Remove the file
                    if let Ok(()) = std::fs::remove_file(&path) {
                        removed_files.push(path);
                    }
                    // Ignore removal errors
                }
            }
        }

        let removed = removed_files;

        if !removed.is_empty() {
            crate::utils::events::send_event(
                ctx,
                Event::OperationCompleted {
                    operation: format!("Removed {} object files", removed.len()),
                    success: true,
                },
            );
        }

        Ok(Report {
            changed_files: removed,
            ..Default::default()
        })
    }
}

impl Patcher for ObjectFileCleaner {}

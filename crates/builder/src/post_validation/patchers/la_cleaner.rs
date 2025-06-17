//! Cleaner that removes libtool archive (.la) files

use crate::post_validation::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::Event;

pub struct LaFileCleaner;

impl crate::post_validation::traits::Action for LaFileCleaner {
    const NAME: &'static str = "Libtool archive cleaner";

    async fn run(ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        let staging_dir = env.staging_dir();
        let mut removed_files = Vec::new();

        // Walk staging directory for .la files
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

            // Check if it's a .la file
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "la" {
                    // Remove the file
                    match std::fs::remove_file(&path) {
                        Ok(()) => {
                            removed_files.push(path);
                        }
                        Err(_) => {
                            // Ignore removal errors
                        }
                    }
                }
            }
        }

        let removed = removed_files;

        if !removed.is_empty() {
            crate::events::send_event(
                ctx,
                Event::OperationCompleted {
                    operation: format!("Removed {} libtool archive files", removed.len()),
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

impl Patcher for LaFileCleaner {}

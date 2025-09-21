//! Cleaner that removes libtool archive (.la) files

use crate::artifact_qa::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;

pub struct LaFileCleaner;

impl crate::artifact_qa::traits::Action for LaFileCleaner {
    const NAME: &'static str = "Libtool archive cleaner";

    async fn run(
        _ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&crate::artifact_qa::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
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
                    if let Ok(()) = std::fs::remove_file(&path) {
                        removed_files.push(path);
                    }
                    // Ignore removal errors
                }
            }
        }

        let mut warnings = Vec::new();
        let removed = removed_files;
        if !removed.is_empty() {
            warnings.push(format!("Removed {} libtool archives", removed.len()));
        }

        Ok(Report {
            changed_files: removed,
            warnings,
            ..Default::default()
        })
    }
}

impl Patcher for LaFileCleaner {}

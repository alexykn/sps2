//! Validator that looks into static archives (*.a) & libtool *.la files.

use crate::post_validation::{
    diagnostics::{DiagnosticCollector, IssueType},
    reports::Report,
    traits::Validator,
};
use crate::{BuildContext, BuildEnvironment};
use object::read::archive::ArchiveFile;
use sps2_errors::Error;
use sps2_events::Event;

pub struct ArchiveScanner;

impl crate::post_validation::traits::Action for ArchiveScanner {
    const NAME: &'static str = "Static‑archive scanner";

    async fn run(ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();

        let mut collector = DiagnosticCollector::new();

        for entry in ignore::WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "a" && ext != "la" {
                continue;
            }

            if ext == "la" {
                // simple text scan for libtool archives
                if let Ok(s) = std::fs::read_to_string(&path) {
                    if s.contains(&build_prefix) {
                        collector.add_finding(
                            crate::post_validation::diagnostics::ValidationFinding {
                                file_path: path,
                                issue_type: IssueType::BuildPathInArchive {
                                    path: build_prefix.clone(),
                                    member: None,
                                },
                                context: std::collections::HashMap::new(),
                            },
                        );
                    }
                }
            } else if let Ok(bytes) = std::fs::read(&path) {
                // Check static archives using the object crate
                if let Ok(archive) = ArchiveFile::parse(&*bytes) {
                    for member in archive.members().flatten() {
                        if let Ok(name) = std::str::from_utf8(member.name()) {
                            if name.contains(&build_prefix) {
                                collector.add_finding(
                                    crate::post_validation::diagnostics::ValidationFinding {
                                        file_path: path.clone(),
                                        issue_type: IssueType::BuildPathInArchive {
                                            path: build_prefix.clone(),
                                            member: Some(name.to_string()),
                                        },
                                        context: std::collections::HashMap::new(),
                                    },
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }

        if collector.has_findings() {
            // Emit detailed diagnostics as warning events
            let diagnostic_messages = collector.generate_diagnostic_messages();

            // Emit each file's diagnostics as a separate warning
            for msg in &diagnostic_messages {
                crate::events::send_event(
                    ctx,
                    Event::Warning {
                        message: "Archive validation failed".to_string(),
                        context: Some(msg.clone()),
                    },
                );
            }

            // Return report with errors (not Err!) so pipeline continues
            let error_count = collector.count();
            let mut report = Report::default();

            // Add the summary as an error so is_fatal() returns true
            report.errors.push(format!(
                "Static archives contain build paths ({} file(s)). Check warnings above for details.",
                error_count
            ));

            Ok(report)
        } else {
            Ok(Report::ok())
        }
    }
}

impl Validator for ArchiveScanner {}

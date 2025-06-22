//! Validator that looks into static archives (*.a) files.

use crate::artifact_qa::{
    diagnostics::{DiagnosticCollector, IssueType},
    reports::Report,
    traits::Validator,
};
use crate::{BuildContext, BuildEnvironment};
use object::read::archive::ArchiveFile;
use sps2_errors::Error;
use sps2_events::Event;

pub struct ArchiveScanner;

impl crate::artifact_qa::traits::Action for ArchiveScanner {
    const NAME: &'static str = "Static‑archive scanner";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&DiagnosticCollector>,
    ) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let build_src = format!("{build_prefix}/src");
        let build_base = "/opt/pm/build";

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
            if ext != "a" {
                continue;
            }

            if let Ok(bytes) = std::fs::read(&path) {
                // Check static archives using the object crate
                if let Ok(archive) = ArchiveFile::parse(&*bytes) {
                    for member in archive.members().flatten() {
                        if let Ok(name) = std::str::from_utf8(member.name()) {
                            if name.contains(build_base) {
                                collector.add_finding(
                                    crate::artifact_qa::diagnostics::ValidationFinding {
                                        file_path: path.clone(),
                                        issue_type: IssueType::BuildPathInArchive {
                                            path: build_base.to_string(),
                                            member: Some(name.to_string()),
                                        },
                                        context: std::collections::HashMap::new(),
                                    },
                                );
                                break;
                            } else if name.contains(&build_prefix) {
                                collector.add_finding(
                                    crate::artifact_qa::diagnostics::ValidationFinding {
                                        file_path: path.clone(),
                                        issue_type: IssueType::BuildPathInArchive {
                                            path: build_prefix.clone(),
                                            member: Some(name.to_string()),
                                        },
                                        context: std::collections::HashMap::new(),
                                    },
                                );
                                break;
                            } else if name.contains(&build_src) {
                                collector.add_finding(
                                    crate::artifact_qa::diagnostics::ValidationFinding {
                                        file_path: path.clone(),
                                        issue_type: IssueType::BuildPathInArchive {
                                            path: build_src.clone(),
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
                crate::utils::events::send_event(
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
                "Static archives contain build paths ({error_count} file(s)). Check warnings above for details."
            ));

            // Include the collector in the report so patchers can use it
            report.findings = Some(collector);
            Ok(report)
        } else {
            Ok(Report::ok())
        }
    }
}

impl Validator for ArchiveScanner {}

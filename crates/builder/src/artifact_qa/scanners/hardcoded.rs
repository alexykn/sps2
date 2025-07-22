//! Validator that searches every byte for `/opt/pm/build/...` or the
//! placeholder prefix. It is binary‑safe and SIMD‑accelerated.

use crate::artifact_qa::{diagnostics::DiagnosticCollector, reports::Report, traits::Validator};
use crate::{BuildContext, BuildEnvironment};
use bstr::ByteSlice;
use ignore::WalkBuilder;
use sps2_errors::Error;
use sps2_events::{AppEvent, GeneralEvent};

pub struct HardcodedScanner;
impl crate::artifact_qa::traits::Action for HardcodedScanner {
    const NAME: &'static str = "Hardcoded‑path scanner";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&DiagnosticCollector>,
    ) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let build_src = format!("{build_prefix}/src");
        let build_base = "/opt/pm/build";

        // Debug: Print the build prefixes we're scanning for
        crate::utils::events::send_event(
            ctx,
            AppEvent::General(GeneralEvent::debug(&format!(
                "Hardcoded path scanner: checking for {build_base} | {build_prefix} | {build_src}"
            ))),
        );

        let mut collector = DiagnosticCollector::new();

        for entry in WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if path.is_file() {
                // Skip Python bytecode files - they contain paths but are regenerated at runtime
                if let Some(ext) = path.extension() {
                    if ext == "pyc" || ext == "pyo" {
                        continue;
                    }
                }
                if let Ok(data) = std::fs::read(&path) {
                    let hay = data.as_slice();

                    // Check for any build-related paths
                    if hay.find(build_base.as_bytes()).is_some() {
                        collector.add_hardcoded_path(&path, build_base, false);
                    } else if hay.find(build_prefix.as_bytes()).is_some() {
                        collector.add_hardcoded_path(&path, &build_prefix, false);
                    } else if hay.find(build_src.as_bytes()).is_some() {
                        collector.add_hardcoded_path(&path, &build_src, false);
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
                    AppEvent::General(GeneralEvent::warning_with_context(
                        "Hardcoded path validation failed",
                        msg,
                    )),
                );
            }

            // Return report with errors (not Err!) so pipeline continues
            let error_count = collector.count();
            let mut report = Report::default();

            // Add the summary as an error so is_fatal() returns true
            report.errors.push(format!(
                "Hardcoded path(s) found in {error_count} file(s). Check warnings above for details."
            ));

            // Include the collector in the report so patchers can use it
            report.findings = Some(collector);
            Ok(report)
        } else {
            Ok(Report::ok())
        }
    }
}
impl Validator for HardcodedScanner {}

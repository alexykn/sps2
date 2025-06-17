//! Validator that searches every byte for `/opt/pm/build/...` or the
//! placeholder prefix. It is binary‑safe and SIMD‑accelerated.

use crate::post_validation::{
    diagnostics::DiagnosticCollector, reports::Report, traits::Validator,
};
use crate::{BuildContext, BuildEnvironment};
use bstr::ByteSlice;
use ignore::WalkBuilder;
use sps2_errors::Error;
use sps2_events::Event;

pub struct HardcodedScanner;
impl crate::post_validation::traits::Action for HardcodedScanner {
    const NAME: &'static str = "Hardcoded‑path scanner";

    async fn run(ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let placeholder = crate::BUILD_PLACEHOLDER_PREFIX;

        let mut collector = DiagnosticCollector::new();

        for entry in WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if path.is_file() {
                if let Ok(data) = std::fs::read(&path) {
                    let hay = data.as_slice();

                    // Check for build prefix
                    if hay.find(build_prefix.as_bytes()).is_some() {
                        collector.add_hardcoded_path(&path, &build_prefix, false);
                    }

                    // Check for placeholder
                    if hay.find(placeholder.as_bytes()).is_some() {
                        collector.add_hardcoded_path(&path, placeholder, true);
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
                        message: "Hardcoded path validation failed".to_string(),
                        context: Some(msg.clone()),
                    },
                );
            }

            // Return report with errors (not Err!) so pipeline continues
            let error_count = collector.count();
            let mut report = Report::default();

            // Add the summary as an error so is_fatal() returns true
            report.errors.push(format!(
                "Hardcoded path(s) found in {} file(s). Check warnings above for details.",
                error_count
            ));

            Ok(report)
        } else {
            Ok(Report::ok())
        }
    }
}
impl Validator for HardcodedScanner {}

//! Re-signs binaries after patching to fix code signature issues on macOS

use crate::artifact_qa::{macho_utils, reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::Event;
use std::path::Path;
use tokio::process::Command;

pub struct CodeSigner;

impl CodeSigner {
    /// Check if a file is a Mach-O binary (executable or dylib)
    fn is_macho_binary(path: &Path) -> bool {
        // Check if it's a dynamic library (including versioned ones)
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.contains(".dylib") || name.contains(".so") {
                return true;
            }
        }

        // Use the shared MachO detection logic
        macho_utils::is_macho_file(path)
    }

    /// Re-sign a binary with ad-hoc signature
    async fn resign_binary(path: &Path) -> Result<bool, std::io::Error> {
        // First check if the signature is valid
        let check = Command::new("codesign")
            .args(["-vvv", &path.to_string_lossy()])
            .output()
            .await?;

        // If signature is invalid or modified, re-sign it
        if check.status.success() {
            Ok(false) // No re-signing needed
        } else {
            let output = Command::new("codesign")
                .args(["-f", "-s", "-", &path.to_string_lossy()])
                .output()
                .await?;

            Ok(output.status.success())
        }
    }
}

impl crate::artifact_qa::traits::Action for CodeSigner {
    const NAME: &'static str = "Code re-signer";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&crate::artifact_qa::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
        // Only run on macOS
        if !cfg!(target_os = "macos") {
            return Ok(Report::ok());
        }

        let mut resigned_count = 0;
        let mut errors = Vec::new();

        // Walk through all files in staging directory
        for entry in ignore::WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if !path.is_file() || !Self::is_macho_binary(&path) {
                continue;
            }

            match Self::resign_binary(&path).await {
                Ok(true) => resigned_count += 1,
                Ok(false) => {} // No re-signing needed
                Err(e) => {
                    errors.push(format!("Failed to re-sign {}: {}", path.display(), e));
                }
            }
        }

        if resigned_count > 0 {
            crate::utils::events::send_event(
                ctx,
                Event::OperationCompleted {
                    operation: format!("Re-signed {resigned_count} binaries"),
                    success: true,
                },
            );
        }

        Ok(Report {
            errors,
            ..Default::default()
        })
    }
}

impl Patcher for CodeSigner {}

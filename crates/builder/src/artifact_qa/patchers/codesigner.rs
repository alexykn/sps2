//! Re-signs binaries after patching to fix code signature issues on macOS

use crate::artifact_qa::{macho_utils, reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::{AppEvent, QaEvent};
use sps2_platform::{PlatformContext, PlatformManager};
use std::path::Path;

pub struct CodeSigner {
    platform: &'static sps2_platform::Platform,
}

impl CodeSigner {
    /// Create a new `CodeSigner` with platform abstraction
    #[must_use]
    pub fn new() -> Self {
        Self {
            platform: PlatformManager::instance().platform(),
        }
    }

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
    async fn resign_binary(
        &self,
        ctx: &PlatformContext,
        path: &Path,
    ) -> Result<bool, sps2_errors::Error> {
        // First check if the signature is valid
        let is_valid =
            (self.platform.binary().verify_signature(ctx, path).await).unwrap_or_default();

        // If signature is invalid or modified, re-sign it
        if is_valid {
            Ok(false) // No re-signing needed
        } else {
            // Re-sign with ad-hoc signature (identity = None)
            match self.platform.binary().sign_binary(ctx, path, None).await {
                Ok(()) => Ok(true),
                Err(e) => Err(e.into()),
            }
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

        let signer = Self::new();

        // Create platform context from build context
        let platform_ctx = signer.platform.create_context(ctx.event_sender.clone());

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

            match signer.resign_binary(&platform_ctx, &path).await {
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
                AppEvent::Qa(QaEvent::CheckCompleted {
                    check_type: "patcher".to_string(),
                    check_name: "codesigner".to_string(),
                    findings_count: resigned_count,
                    severity_counts: std::collections::HashMap::new(),
                }),
            );
        }

        Ok(Report {
            errors,
            ..Default::default()
        })
    }
}

impl Default for CodeSigner {
    fn default() -> Self {
        Self::new()
    }
}

impl Patcher for CodeSigner {}

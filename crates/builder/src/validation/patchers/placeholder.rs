//! Replaces `BUILD_PLACEHOLDER` and build‑prefix strings in *text* files.

use crate::validation::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::Event;

use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;

pub struct PlaceholderPatcher;
impl crate::validation::traits::Action for PlaceholderPatcher {
    const NAME: &'static str = "Placeholder / build‑path replacer";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        findings: Option<&crate::validation::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
        use std::collections::HashSet;
        use std::fs::{self, File};
        use std::io::{BufWriter, Read, Write};

        let actual_prefix = "/opt/pm/live";
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let build_src = format!("{build_prefix}/src");
        let build_base = "/opt/pm/build";
        // Replace any build paths with actual prefix

        // ----------- build the globset of *binary* extensions we skip -----------
        let mut gsb = GlobSetBuilder::new();
        for pat in &[
            "*.png", "*.jpg", "*.jpeg", "*.gif", "*.ico", "*.gz", "*.bz2", "*.xz", "*.zip",
            "*.tar", "*.a", "*.so", "*.dylib", "*.o", "*.rlib",
        ] {
            gsb.add(Glob::new(pat).unwrap());
        }
        let binaries = gsb.build().unwrap();

        let mut changed = Vec::new();

        // Get the list of files to process
        let files_to_process: Box<dyn Iterator<Item = std::path::PathBuf>> =
            if let Some(findings) = findings {
                // Use validator findings - only process files with hardcoded paths
                let files_with_issues = findings.get_files_with_hardcoded_paths();
                let paths: HashSet<std::path::PathBuf> =
                    files_with_issues.keys().map(|&p| p.to_path_buf()).collect();
                Box::new(paths.into_iter())
            } else {
                // Fall back to walking the entire directory (old behavior)
                Box::new(
                    WalkBuilder::new(env.staging_dir())
                        .hidden(false)
                        .parents(false)
                        .build()
                        .filter_map(Result::ok)
                        .map(ignore::DirEntry::into_path)
                        .filter(|p| p.is_file()),
                )
            };

        for path in files_to_process {
            // Skip binary files based on extension
            if binaries.is_match(&path) {
                continue;
            }

            if let Ok(mut f) = File::open(&path) {
                let mut buf = Vec::new();
                if f.read_to_end(&mut buf).is_ok() {
                    if let Ok(txt) = String::from_utf8(buf) {
                        let mut modified = false;
                        let mut result = txt.clone();

                        // Replace build paths in order of specificity (most specific first)
                        if result.contains(&build_src) {
                            result = result.replace(&build_src, actual_prefix);
                            modified = true;
                        }
                        if result.contains(&build_prefix) {
                            result = result.replace(&build_prefix, actual_prefix);
                            modified = true;
                        }
                        if result.contains(build_base) {
                            result = result.replace(build_base, actual_prefix);
                            modified = true;
                        }

                        if modified {
                            let _ = fs::create_dir_all(path.parent().unwrap());
                            if let Ok(file) = File::create(&path) {
                                let mut writer = BufWriter::new(file);
                                if writer.write_all(result.as_bytes()).is_ok() {
                                    changed.push(path);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Event message
        if !changed.is_empty() {
            crate::utils::events::send_event(
                ctx,
                Event::OperationCompleted {
                    operation: format!("Rewrote {} files", changed.len()),
                    success: true,
                },
            );
        }

        Ok(Report {
            changed_files: changed,
            ..Default::default()
        })
    }
}
impl Patcher for PlaceholderPatcher {}

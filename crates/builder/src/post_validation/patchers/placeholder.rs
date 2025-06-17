//! Replaces BUILD_PLACEHOLDER and build‑prefix strings in *text* files.

use crate::post_validation::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::Event;

use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;

pub struct PlaceholderPatcher;
impl crate::post_validation::traits::Action for PlaceholderPatcher {
    const NAME: &'static str = "Placeholder / build‑path replacer";

    async fn run(ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        use std::fs::{self, File};
        use std::io::{BufWriter, Read, Write};

        let actual_prefix = "/opt/pm/live";
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let placeholder = crate::BUILD_PLACEHOLDER_PREFIX;

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

        for entry in WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
        {
            let path = match entry {
                Ok(d) => d.into_path(),
                Err(_) => continue,
            };

            if path.is_file() && !binaries.is_match(&path) {
                if let Ok(mut f) = File::open(&path) {
                    let mut buf = Vec::new();
                    if f.read_to_end(&mut buf).is_ok() {
                        if let Ok(txt) = String::from_utf8(buf) {
                            if txt.contains(placeholder) || txt.contains(&build_prefix) {
                                let replaced = txt
                                    .replace(placeholder, actual_prefix)
                                    .replace(&build_prefix, actual_prefix);
                                let _ = fs::create_dir_all(path.parent().unwrap());
                                if let Ok(file) = File::create(&path) {
                                    let mut writer = BufWriter::new(file);
                                    if writer.write_all(replaced.as_bytes()).is_ok() {
                                        changed.push(path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Event message
        if !changed.is_empty() {
            crate::events::send_event(
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

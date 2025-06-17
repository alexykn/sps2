//! Fixes install‑name / LC_RPATH of Mach‑O dylibs & executables.

use crate::post_validation::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::Event;

use ignore::WalkBuilder;
use tokio::process::Command;

pub struct RPathPatcher;
impl crate::post_validation::traits::Action for RPathPatcher {
    const NAME: &'static str = "install_name_tool patcher";

    async fn run(ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        // Collect binaries to patch (walk is sync but cheap)
        let files: Vec<_> = WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
            .map(ignore::DirEntry::into_path)
            .filter(|p| {
                if !p.is_file() {
                    return false;
                }

                // Check by extension first
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    if ["dylib", "so"].contains(&ext) {
                        return true;
                    }
                }

                // Check if it's a Mach-O executable (no extension)
                if let Ok(data) = std::fs::read(p) {
                    if data.len() >= 4 {
                        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                        // MH_MAGIC_64 (0xfeedfacf) or MH_MAGIC (0xfeedface)
                        return magic == 0xfeed_facf || magic == 0xfeed_face;
                    }
                }

                false
            })
            .collect();

        let lib_path = "/opt/pm/live/lib".to_string(); // Actual runtime lib path
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let placeholder = crate::BUILD_PLACEHOLDER_PREFIX;

        let mut changed = Vec::new();

        // Process each file sequentially
        for path in files {
            let path_s = path.to_string_lossy().into_owned();
            // Use otool to inspect
            let out = Command::new("otool").args(["-l", &path_s]).output().await;
            let Ok(out) = out else {
                continue;
            };
            if !out.status.success() {
                continue;
            }
            let text = String::from_utf8_lossy(&out.stdout);

            // gather bad rpaths
            let mut bad_rpaths = Vec::new();
            let mut need_good = false;
            let mut lines = text.lines();
            while let Some(l) = lines.next() {
                if l.contains("LC_RPATH") {
                    let _ = lines.next(); // skip cmdsize
                    if let Some(p) = lines.next() {
                        if let Some(idx) = p.find("path ") {
                            let r = &p[idx + 5..p.find(" (").unwrap_or(p.len())];
                            if r == lib_path {
                                need_good = false;
                            } else if r.contains(&build_prefix) || r.contains(placeholder) {
                                bad_rpaths.push(r.to_owned());
                            }
                        }
                    }
                } else if l.contains("@rpath/") {
                    need_good = true;
                }
            }

            // do the work
            if need_good {
                let _ = Command::new("install_name_tool")
                    .args(["-add_rpath", &lib_path, &path_s])
                    .output()
                    .await;
            }
            for bad in &bad_rpaths {
                let _ = Command::new("install_name_tool")
                    .args(["-delete_rpath", bad, &path_s])
                    .output()
                    .await;
            }
            if need_good || !bad_rpaths.is_empty() {
                changed.push(path.clone());
            }
        }

        if !changed.is_empty() {
            crate::events::send_event(
                ctx,
                Event::OperationCompleted {
                    operation: format!("Fixed RPATH in {} binaries", changed.len()),
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
impl Patcher for RPathPatcher {}

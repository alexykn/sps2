//! Fixes *.pc and *Config.cmake so downstream builds never see /opt/pm/build/…

use crate::artifact_qa::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use ignore::WalkBuilder;
use sps2_errors::Error;

pub struct PkgConfigPatcher;
impl crate::artifact_qa::traits::Action for PkgConfigPatcher {
    const NAME: &'static str = "pkg‑config / CMake patcher";

    async fn run(
        _ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&crate::artifact_qa::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let build_src = format!("{build_prefix}/src");
        let build_base = "/opt/pm/build";
        let actual = "/opt/pm/live";

        let pat = WalkBuilder::new(env.staging_dir())
            .build()
            .filter_map(Result::ok)
            .map(ignore::DirEntry::into_path)
            .filter(|p| {
                p.is_file() && {
                    p.extension().and_then(|e| e.to_str()) == Some("pc")
                        || p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.ends_with("Config.cmake"))
                }
            })
            .collect::<Vec<_>>();

        let mut changed = Vec::new();
        for f in pat {
            if let Ok(s) = std::fs::read_to_string(&f) {
                let mut modified = false;
                let mut result = s.clone();

                // Replace build paths in order of specificity (most specific first)
                if result.contains(&build_src) {
                    result = result.replace(&build_src, actual);
                    modified = true;
                }
                if result.contains(&build_prefix) {
                    result = result.replace(&build_prefix, actual);
                    modified = true;
                }
                if result.contains(build_base) {
                    result = result.replace(build_base, actual);
                    modified = true;
                }

                if modified {
                    std::fs::write(&f, result)?;
                    changed.push(f);
                }
            }
        }

        Ok(Report {
            changed_files: changed,
            ..Default::default()
        })
    }
}
impl Patcher for PkgConfigPatcher {}

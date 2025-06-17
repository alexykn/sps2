//! Fixes *.pc and *Config.cmake so downstream builds never see /opt/pm/build/…

use crate::post_validation::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use ignore::WalkBuilder;
use sps2_errors::Error;

pub struct PkgConfigPatcher;
impl crate::post_validation::traits::Action for PkgConfigPatcher {
    const NAME: &'static str = "pkg‑config / CMake patcher";

    async fn run(_ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
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
                            .map(|n| n.ends_with("Config.cmake"))
                            .unwrap_or(false)
                }
            })
            .collect::<Vec<_>>();

        let mut changed = Vec::new();
        for f in pat {
            if let Ok(s) = std::fs::read_to_string(&f) {
                if s.contains(&build_prefix) {
                    let t = s.replace(&build_prefix, actual);
                    std::fs::write(&f, t)?;
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

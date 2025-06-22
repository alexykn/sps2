//! Converts absolute include paths in headers to <relative> form.

use crate::validation::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use ignore::WalkBuilder;
use regex::Regex;
use sps2_errors::Error;

pub struct HeaderPatcher;
impl crate::validation::traits::Action for HeaderPatcher {
    const NAME: &'static str = "Header include‑fixer";

    async fn run(
        _ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&crate::validation::diagnostics::DiagnosticCollector>,
    ) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let build_src = format!("{}/src", build_prefix);
        let build_base = "/opt/pm/build";

        // Create regex for all build paths
        let re = Regex::new(&format!(
            r#"#\s*include\s*"({}|{}|{})[^"]+""#,
            regex::escape(&build_src),
            regex::escape(&build_prefix),
            regex::escape(build_base)
        ))
        .unwrap();

        let mut changed = Vec::new();
        for dir in ["include", "Headers"] {
            let root = env.staging_dir().join(dir);
            if !root.exists() {
                continue;
            }
            for entry in WalkBuilder::new(&root).build().flatten() {
                let p = entry.into_path();
                if p.is_file() {
                    if let Ok(src) = std::fs::read_to_string(&p) {
                        if re.is_match(&src) {
                            let repl = re.replace_all(&src, |caps: &regex::Captures| {
                                // naive: just strip the prefix and keep quotes
                                let full = &caps.get(0).unwrap().as_str()[0..];
                                let inner = full.trim_start_matches("#include ").trim();
                                let stripped = inner
                                    .trim_matches('"')
                                    .trim_start_matches(&build_src)
                                    .trim_start_matches(&build_prefix)
                                    .trim_start_matches(build_base)
                                    .trim_start_matches('/');
                                format!("#include \"{}\"", stripped)
                            });
                            std::fs::write(&p, repl.as_bytes())?;
                            changed.push(p);
                        }
                    }
                }
            }
        }
        Ok(Report {
            changed_files: changed,
            ..Default::default()
        })
    }
}
impl Patcher for HeaderPatcher {}

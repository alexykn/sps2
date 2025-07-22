//! Validator that inspects Mach‑O headers without spawning `otool`.

use crate::artifact_qa::{
    diagnostics::{DiagnosticCollector, IssueType},
    reports::Report,
    traits::Validator,
};
use crate::{BuildContext, BuildEnvironment};
use object::{
    macho::{MachHeader32, MachHeader64},
    read::macho::{
        FatArch, LoadCommandVariant, MachHeader, MachOFatFile32, MachOFatFile64, MachOFile,
    },
    Endianness, FileKind,
};
use sps2_errors::Error;
use sps2_events::AppEvent;

pub struct MachOScanner;

impl crate::artifact_qa::traits::Action for MachOScanner {
    const NAME: &'static str = "Mach‑O load‑command scanner";

    async fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        _findings: Option<&DiagnosticCollector>,
    ) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let build_src = format!("{build_prefix}/src");
        let build_base = "/opt/pm/build";

        let mut collector = DiagnosticCollector::new();

        for entry in ignore::WalkBuilder::new(env.staging_dir())
            .hidden(false)
            .parents(false)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.into_path();
            if !path.is_file() {
                continue;
            }

            if let Ok(data) = std::fs::read(&path) {
                if let Ok(kind) = FileKind::parse(&*data) {
                    let build_paths = vec![build_base, &build_prefix, &build_src];
                    check_macho_file(&data, kind, &build_paths, &path, &mut collector);
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
                    Event::Warning {
                        message: "Mach-O validation failed".to_string(),
                        context: Some(msg.clone()),
                    },
                );
            }

            // Return report with errors (not Err!) so pipeline continues
            let error_count = collector.count();
            let mut report = Report::default();

            // Add the summary as an error so is_fatal() returns true
            report.errors.push(format!(
                "Mach‑O contains bad install‑name or RPATH ({error_count} file(s)). Check warnings above for details."
            ));

            // Include the collector in the report so patchers can use it
            report.findings = Some(collector);
            Ok(report)
        } else {
            Ok(Report::ok())
        }
    }
}

fn check_macho_file(
    data: &[u8],
    kind: FileKind,
    build_paths: &[&str],
    file_path: &std::path::Path,
    collector: &mut DiagnosticCollector,
) {
    match kind {
        FileKind::MachO32 => {
            if let Ok(file) = MachOFile::<MachHeader32<Endianness>, _>::parse(data) {
                check_load_commands(&file, build_paths, file_path, collector);
            }
        }
        FileKind::MachO64 => {
            if let Ok(file) = MachOFile::<MachHeader64<Endianness>, _>::parse(data) {
                check_load_commands(&file, build_paths, file_path, collector);
            }
        }
        FileKind::MachOFat32 => {
            if let Ok(fat) = MachOFatFile32::parse(data) {
                for arch in fat.arches() {
                    let (off, sz) = arch.file_range();
                    let Ok(start): Result<usize, _> = off.try_into() else {
                        continue;
                    };
                    let Ok(size): Result<usize, _> = sz.try_into() else {
                        continue;
                    };
                    let Some(end) = start.checked_add(size) else {
                        continue;
                    };
                    if let Some(slice) = data.get(start..end) {
                        if let Ok(kind) = FileKind::parse(slice) {
                            check_macho_file(slice, kind, build_paths, file_path, collector);
                        }
                    }
                }
            }
        }
        FileKind::MachOFat64 => {
            if let Ok(fat) = MachOFatFile64::parse(data) {
                for arch in fat.arches() {
                    let (off, sz) = arch.file_range();
                    let Ok(start): Result<usize, _> = off.try_into() else {
                        continue;
                    };
                    let Ok(size): Result<usize, _> = sz.try_into() else {
                        continue;
                    };
                    let Some(end) = start.checked_add(size) else {
                        continue;
                    };
                    if let Some(slice) = data.get(start..end) {
                        if let Ok(kind) = FileKind::parse(slice) {
                            check_macho_file(slice, kind, build_paths, file_path, collector);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn check_load_commands<'data, Mach, R>(
    file: &MachOFile<'data, Mach, R>,
    build_paths: &[&str],
    file_path: &std::path::Path,
    collector: &mut DiagnosticCollector,
) where
    Mach: MachHeader,
    R: object::ReadRef<'data>,
{
    let endian = file.endian();
    if let Ok(mut commands) = file.macho_load_commands() {
        while let Ok(Some(cmd)) = commands.next() {
            if let Ok(variant) = cmd.variant() {
                let path_bytes = match variant {
                    LoadCommandVariant::Dylib(d) | LoadCommandVariant::IdDylib(d) => {
                        cmd.string(endian, d.dylib.name).ok()
                    }
                    LoadCommandVariant::Rpath(r) => cmd.string(endian, r.path).ok(),
                    _ => None,
                };

                if let Some(bytes) = path_bytes {
                    if let Ok(path_str) = std::str::from_utf8(bytes) {
                        // Check for any build paths - those are always bad
                        // /opt/pm/live paths are always OK
                        for build_path in build_paths {
                            if path_str.starts_with(build_path) {
                                let issue_type = match variant {
                                    LoadCommandVariant::Rpath(_) => IssueType::BadRPath {
                                        rpath: path_str.to_string(),
                                    },
                                    _ => IssueType::BadInstallName {
                                        install_name: path_str.to_string(),
                                    },
                                };
                                collector.add_macho_issue(file_path, issue_type);
                                break; // One match is enough
                            }
                        }

                        // Note: We intentionally do NOT check for self-referencing install names
                        // when they use @rpath/. This is the correct, modern way to build
                        // relocatable libraries on macOS. The @rpath/ prefix tells the dynamic
                        // linker to search for the library using the runtime path search paths.
                        //
                        // Example: A library with install name "@rpath/libfoo.1.dylib" is
                        // correctly configured for runtime path loading and should not be flagged
                        // as an error.
                    }
                }
            }
        }
    }
}

impl Validator for MachOScanner {}

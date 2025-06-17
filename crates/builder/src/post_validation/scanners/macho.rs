//! Validator that inspects Mach‑O headers without spawning `otool`.

use crate::post_validation::{
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
use sps2_events::Event;

pub struct MachOScanner;

impl crate::post_validation::traits::Action for MachOScanner {
    const NAME: &'static str = "Mach‑O load‑command scanner";

    async fn run(ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let placeholder = crate::BUILD_PLACEHOLDER_PREFIX;

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
                    check_macho_file(
                        &data,
                        kind,
                        &build_prefix,
                        placeholder,
                        &path,
                        &mut collector,
                    );
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
                "Mach‑O contains bad install‑name or RPATH ({} file(s)). Check warnings above for details.",
                error_count
            ));

            Ok(report)
        } else {
            Ok(Report::ok())
        }
    }
}

fn check_macho_file(
    data: &[u8],
    kind: FileKind,
    build_prefix: &str,
    placeholder: &str,
    file_path: &std::path::Path,
    collector: &mut DiagnosticCollector,
) {
    match kind {
        FileKind::MachO32 => {
            if let Ok(file) = MachOFile::<MachHeader32<Endianness>, _>::parse(data) {
                check_load_commands(&file, build_prefix, placeholder, file_path, collector);
                return;
            }
        }
        FileKind::MachO64 => {
            if let Ok(file) = MachOFile::<MachHeader64<Endianness>, _>::parse(data) {
                check_load_commands(&file, build_prefix, placeholder, file_path, collector);
                return;
            }
        }
        FileKind::MachOFat32 => {
            if let Ok(fat) = MachOFatFile32::parse(data) {
                for arch in fat.arches() {
                    let (off, sz) = arch.file_range();
                    if let Some(slice) = data.get(off as usize..(off + sz) as usize) {
                        if let Ok(kind) = FileKind::parse(slice) {
                            check_macho_file(
                                slice,
                                kind,
                                build_prefix,
                                placeholder,
                                file_path,
                                collector,
                            );
                        }
                    }
                }
            }
        }
        FileKind::MachOFat64 => {
            if let Ok(fat) = MachOFatFile64::parse(data) {
                for arch in fat.arches() {
                    let (off, sz) = arch.file_range();
                    if let Some(slice) = data.get(off as usize..(off + sz) as usize) {
                        if let Ok(kind) = FileKind::parse(slice) {
                            check_macho_file(
                                slice,
                                kind,
                                build_prefix,
                                placeholder,
                                file_path,
                                collector,
                            );
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
    build_prefix: &str,
    placeholder: &str,
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
                        if path_str.starts_with(build_prefix) || path_str.contains(placeholder) {
                            let issue_type = match variant {
                                LoadCommandVariant::Rpath(_) => IssueType::BadRPath {
                                    rpath: path_str.to_string(),
                                },
                                _ => IssueType::BadInstallName {
                                    install_name: path_str.to_string(),
                                },
                            };
                            collector.add_macho_issue(file_path, issue_type);
                        }
                    }
                }
            }
        }
    }
}

impl Validator for MachOScanner {}

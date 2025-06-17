//! Binary-safe string patcher for embedded paths in executables and libraries

use crate::post_validation::{reports::Report, traits::Patcher};
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_events::Event;
use std::collections::HashMap;
use std::path::Path;

/// Find all occurrences of needle in haystack and return their byte offsets
fn find_binary_strings(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    if needle.is_empty() || haystack.len() < needle.len() {
        return positions;
    }

    // Use a simple sliding window approach
    for i in 0..=(haystack.len() - needle.len()) {
        if &haystack[i..i + needle.len()] == needle {
            positions.push(i);
        }
    }

    positions
}

/// Replace a string in binary data with null-padding to maintain file structure
/// Returns true if replacement was made, false if new string was too long
fn replace_binary_string(
    data: &mut [u8],
    offset: usize,
    old_str: &str,
    new_str: &str,
    allocated_len: Option<usize>,
) -> bool {
    let old_bytes = old_str.as_bytes();
    let new_bytes = new_str.as_bytes();

    // Determine allocated length by scanning for null terminator
    let alloc_len = if let Some(len) = allocated_len {
        len
    } else {
        // Find the null terminator starting from offset
        let mut len = old_bytes.len();
        for (i, &byte) in data.iter().enumerate().skip(offset + old_bytes.len()) {
            if byte == 0 {
                len = i - offset + 1; // Include the null terminator
                break;
            }
        }
        len
    };

    // Check if new string fits in allocated space
    if new_bytes.len() + 1 > alloc_len {
        return false;
    }

    // Copy new string
    data[offset..offset + new_bytes.len()].copy_from_slice(new_bytes);

    // Null-pad the rest
    for i in (offset + new_bytes.len())..(offset + alloc_len) {
        if i < data.len() {
            data[i] = 0;
        }
    }

    true
}

pub struct BinaryStringPatcher;

impl crate::post_validation::traits::Action for BinaryStringPatcher {
    const NAME: &'static str = "Binary string patcher";

    async fn run(ctx: &BuildContext, env: &BuildEnvironment) -> Result<Report, Error> {
        let staging_dir = env.staging_dir();
        let build_prefix = env.build_prefix().to_string_lossy().into_owned();
        let install_prefix = "/opt/pm/live".to_string(); // Actual runtime installation prefix
        let placeholder = crate::BUILD_PLACEHOLDER_PREFIX;

        // Prepare replacements map
        let mut replacements = HashMap::new();
        replacements.insert(build_prefix.clone(), install_prefix.clone());
        replacements.insert(placeholder.to_string(), install_prefix.clone());

        let mut patched_files = Vec::new();
        let mut skipped_files = Vec::new();

        // Walk staging directory for binary files
        for entry in ignore::WalkBuilder::new(staging_dir)
            .hidden(false)
            .parents(false)
            .build()
        {
            let path = match entry {
                Ok(e) => e.into_path(),
                Err(_) => continue,
            };

            if !path.is_file() {
                continue;
            }

            // Check if it's a binary file we should process
            let should_process = if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                ["so", "dylib", "a"].contains(&ext)
            } else {
                // Check for Mach-O magic bytes
                if let Ok(data) = std::fs::read(&path) {
                    if data.len() >= 4 {
                        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                        magic == 0xfeed_facf || magic == 0xfeed_face
                    } else {
                        false
                    }
                } else {
                    false
                }
            };

            if !should_process {
                continue;
            }

            // Process the file
            if let Ok((was_patched, was_skipped)) = process_binary_file(&path, &replacements) {
                if was_patched {
                    patched_files.push(path.clone());
                }
                if was_skipped {
                    skipped_files.push((path, "Path too long".to_string()));
                }
            }
        }

        let patched = patched_files;
        let skipped = skipped_files;

        if !skipped.is_empty() {
            // Send warning event about skipped files
            crate::events::send_event(
                ctx,
                Event::Warning {
                    message: format!(
                        "Binary string patcher: {} paths too long to patch in {} files",
                        skipped.len(),
                        skipped
                            .iter()
                            .map(|(p, _)| p)
                            .collect::<std::collections::HashSet<_>>()
                            .len()
                    ),
                    context: Some(
                        "Some embedded paths could not be patched due to length constraints"
                            .to_string(),
                    ),
                },
            );
        }

        if !patched.is_empty() {
            crate::events::send_event(
                ctx,
                Event::OperationCompleted {
                    operation: format!("Patched {} binary files", patched.len()),
                    success: true,
                },
            );
        }

        Ok(Report {
            changed_files: patched,
            ..Default::default()
        })
    }
}

fn process_binary_file(
    path: &Path,
    replacements: &HashMap<String, String>,
) -> Result<(bool, bool), Error> {
    let mut data = std::fs::read(path)?;
    let mut any_patched = false;
    let mut any_skipped = false;

    for (old_path, new_path) in replacements {
        let positions = find_binary_strings(&data, old_path.as_bytes());

        for offset in positions {
            if replace_binary_string(&mut data, offset, old_path, new_path, None) {
                any_patched = true;
            } else {
                any_skipped = true;
            }
        }
    }

    if any_patched {
        // Write the patched file atomically
        let temp_path = path.with_extension("tmp");
        std::fs::write(&temp_path, &data)?;
        std::fs::rename(&temp_path, path)?;
    }

    Ok((any_patched, any_skipped))
}

impl Patcher for BinaryStringPatcher {}

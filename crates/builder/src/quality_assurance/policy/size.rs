//! Binary and package size limit validation

use super::PolicyValidator;
use crate::quality_assurance::types::{PolicyRule, QaCheck, QaCheckType, QaSeverity};
use crate::BuildContext;
use sps2_errors::Error;
use std::path::Path;
use tokio::fs;

/// Size limit validator
pub struct SizeValidator;

impl SizeValidator {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for SizeValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PolicyValidator for SizeValidator {
    fn id(&self) -> &'static str {
        "binary-size"
    }

    fn name(&self) -> &'static str {
        "Binary Size Limit"
    }

    async fn validate(
        &self,
        _context: &BuildContext,
        path: &Path,
        rule: &PolicyRule,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut checks = Vec::new();

        // Parse configuration
        let config = SizeConfig::from_rule(rule);

        // Scan for binary files and check sizes
        let (binary_files, total_size) = find_binary_files(path).await?;

        // Check total package size
        checks.extend(self.check_total_size(total_size, &config, rule.severity));

        // Check individual binary sizes
        checks.extend(
            self.check_binary_sizes(&binary_files, &config, rule.severity)
                .await?,
        );

        Ok(checks)
    }
}

impl SizeValidator {
    /// Check total package size against limits
    fn check_total_size(
        &self,
        total_size: u64,
        config: &SizeConfig,
        severity: QaSeverity,
    ) -> Vec<QaCheck> {
        let mut checks = Vec::new();

        if total_size > config.max_total_size_bytes {
            checks.push(
                QaCheck::new(
                    QaCheckType::SizeLimit,
                    "package-size",
                    severity,
                    format!(
                        "Total package size ({:.2} MB) exceeds limit ({:.2} MB)",
                        bytes_to_mb(total_size),
                        config.max_total_size_mb
                    ),
                )
                .with_context("Consider optimizing assets or splitting into multiple packages"),
            );
        } else if total_size > config.warn_total_size_bytes {
            checks.push(QaCheck::new(
                QaCheckType::SizeLimit,
                "package-size",
                QaSeverity::Warning,
                format!(
                    "Total package size ({:.2} MB) is approaching limit ({:.2} MB)",
                    bytes_to_mb(total_size),
                    config.max_total_size_mb
                ),
            ));
        }

        checks
    }

    /// Check individual binary file sizes
    async fn check_binary_sizes(
        &self,
        binary_files: &[(std::path::PathBuf, u64)],
        config: &SizeConfig,
        severity: QaSeverity,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut checks = Vec::new();

        for (binary_path, size) in binary_files {
            checks.extend(self.check_single_binary_size(binary_path, *size, config, severity));

            // Check for debug symbols if enabled
            if config.check_debug_symbols && has_debug_symbols(binary_path).await? {
                checks.push(self.create_debug_symbols_check(binary_path));
            }
        }

        Ok(checks)
    }

    /// Check size of a single binary
    fn check_single_binary_size(
        &self,
        binary_path: &Path,
        size: u64,
        config: &SizeConfig,
        severity: QaSeverity,
    ) -> Vec<QaCheck> {
        let mut checks = Vec::new();

        if size > config.max_binary_size_bytes {
            checks.push(
                QaCheck::new(
                    QaCheckType::SizeLimit,
                    "binary-size",
                    severity,
                    format!(
                        "Binary size ({:.2} MB) exceeds limit ({:.2} MB)",
                        bytes_to_mb(size),
                        config.max_binary_size_mb
                    ),
                )
                .with_location(binary_path.to_path_buf(), None, None)
                .with_context("Consider enabling optimizations or stripping debug symbols"),
            );
        } else if size > config.warn_binary_size_bytes {
            checks.push(
                QaCheck::new(
                    QaCheckType::SizeLimit,
                    "binary-size",
                    QaSeverity::Warning,
                    format!(
                        "Binary size ({:.2} MB) is approaching limit ({:.2} MB)",
                        bytes_to_mb(size),
                        config.max_binary_size_mb
                    ),
                )
                .with_location(binary_path.to_path_buf(), None, None),
            );
        }

        checks
    }

    /// Create check for debug symbols
    fn create_debug_symbols_check(&self, binary_path: &Path) -> QaCheck {
        QaCheck::new(
            QaCheckType::SizeLimit,
            "binary-size",
            QaSeverity::Info,
            "Binary contains debug symbols",
        )
        .with_location(binary_path.to_path_buf(), None, None)
        .with_context("Debug symbols increase binary size. Consider stripping for release builds")
    }
}

/// Configuration for size checks
struct SizeConfig {
    max_binary_size_mb: f64,
    max_total_size_mb: f64,
    max_binary_size_bytes: u64,
    max_total_size_bytes: u64,
    warn_binary_size_bytes: u64,
    warn_total_size_bytes: u64,
    check_debug_symbols: bool,
}

impl SizeConfig {
    /// Create configuration from policy rule
    fn from_rule(rule: &PolicyRule) -> Self {
        let max_binary_size_mb = rule
            .config
            .get("max_size_mb")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(100.0);

        let max_total_size_mb = rule
            .config
            .get("max_total_size_mb")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(500.0);

        let warn_threshold_percent = rule
            .config
            .get("warn_threshold_percent")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(80.0);

        let check_debug_symbols = rule
            .config
            .get("check_debug_symbols")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        let max_binary_size_bytes = mb_to_bytes(max_binary_size_mb);
        let max_total_size_bytes = mb_to_bytes(max_total_size_mb);
        let warn_binary_size_bytes =
            (max_binary_size_bytes as f64 * warn_threshold_percent / 100.0) as u64;
        let warn_total_size_bytes =
            (max_total_size_bytes as f64 * warn_threshold_percent / 100.0) as u64;

        Self {
            max_binary_size_mb,
            max_total_size_mb,
            max_binary_size_bytes,
            max_total_size_bytes,
            warn_binary_size_bytes,
            warn_total_size_bytes,
            check_debug_symbols,
        }
    }
}

/// Convert megabytes to bytes
fn mb_to_bytes(mb: f64) -> u64 {
    (mb * 1024.0 * 1024.0) as u64
}

/// Convert bytes to megabytes
fn bytes_to_mb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

/// Find all binary files in a directory and calculate total size
async fn find_binary_files(dir: &Path) -> Result<(Vec<(std::path::PathBuf, u64)>, u64), Error> {
    let mut binary_files = Vec::new();
    let mut total_size = 0u64;

    find_binary_files_recursive(dir, &mut binary_files, &mut total_size).await?;

    Ok((binary_files, total_size))
}

/// Recursively find binary files
fn find_binary_files_recursive<'a>(
    dir: &'a Path,
    binary_files: &'a mut Vec<(std::path::PathBuf, u64)>,
    total_size: &'a mut u64,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>> {
    Box::pin(async move {
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = entry.metadata().await?;

            if metadata.is_file() {
                let size = metadata.len();
                *total_size += size;

                // Check if it's likely a binary file
                if is_likely_binary(&path).await {
                    binary_files.push((path, size));
                }
            } else if metadata.is_dir() {
                // Skip common directories that shouldn't count
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, ".git" | "node_modules" | "target" | "build") {
                        continue;
                    }
                }

                find_binary_files_recursive(&path, binary_files, total_size).await?;
            }
        }

        Ok(())
    })
}

/// Check if a file is likely a binary executable
async fn is_likely_binary(path: &Path) -> bool {
    // Check by extension first
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if matches!(ext, "exe" | "dll" | "so" | "dylib" | "a" | "o") {
            return true;
        }
    }

    // Check if file has no extension (common for Unix executables)
    if path.extension().is_none() {
        // Check if executable bit is set
        if let Ok(metadata) = fs::metadata(path).await {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 != 0 {
                    // Read first few bytes to check for binary signatures
                    if let Ok(mut file) = fs::File::open(path).await {
                        use tokio::io::AsyncReadExt;
                        let mut buffer = [0u8; 4];
                        if file.read_exact(&mut buffer).await.is_ok() {
                            // Check for common binary signatures
                            return matches!(
                                &buffer,
                                [0x7f, b'E', b'L', b'F'] | // ELF
                                [b'M', b'Z', _, _] |        // PE/COFF
                                [0xfe, 0xed, 0xfa, 0xce | 0xcf] | // Mach-O 32/64-bit
                                [0xce | 0xcf, 0xfa, 0xed, 0xfe] // Mach-O (swapped)
                            );
                        }
                    }
                }
            }
        }
    }

    false
}

/// Check if a binary has debug symbols (basic check)
async fn has_debug_symbols(path: &Path) -> Result<bool, Error> {
    // This is a simplified check - in production, you'd use proper binary analysis tools

    // For now, just check file size as a heuristic
    // Debug binaries are typically much larger
    if let Ok(metadata) = fs::metadata(path).await {
        let size = metadata.len();

        // Check for .debug or .dSYM files alongside the binary
        let debug_file = path.with_extension("debug");
        let dsym_dir = path.with_extension("dSYM");

        if debug_file.exists() || dsym_dir.exists() {
            return Ok(true);
        }

        // Heuristic: very large binaries often have debug symbols
        // This is not accurate but serves as a placeholder
        if size > 50 * 1024 * 1024 {
            // > 50MB
            return Ok(true);
        }
    }

    Ok(false)
}

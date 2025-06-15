//! Archive extraction utilities

use crate::Result;
use async_compression::tokio::bufread::{BzDecoder, GzipDecoder, XzDecoder};
use sps2_errors::BuildError;
use std::path::{Path, PathBuf};
use tar::Archive as TarArchive;
use tokio::io::{AsyncRead, BufReader};

/// Extract an archive to a destination directory
pub async fn extract(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    // Ensure destination directory exists
    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| BuildError::DraftSourceFailed {
            message: format!("Failed to create destination directory: {e}"),
        })?;

    let archive_path = archive_path.to_path_buf();
    let dest_dir = dest_dir.to_path_buf();

    // Extract based on file extension
    match archive_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("gz" | "tgz") => {
            // Check if it's a .tar.gz
            if archive_path.to_string_lossy().ends_with(".tar.gz")
                || archive_path.extension() == Some(std::ffi::OsStr::new("tgz"))
            {
                extract_tar_gz(archive_path, dest_dir).await
            } else {
                Err(BuildError::DraftSourceFailed {
                    message: "Unsupported archive format: plain .gz files".to_string(),
                }
                .into())
            }
        }
        Some("tar") => extract_tar(archive_path, dest_dir).await,
        Some("zip") => extract_zip(archive_path, dest_dir).await,
        Some("bz2" | "tbz2") => {
            if archive_path.to_string_lossy().ends_with(".tar.bz2")
                || archive_path.extension() == Some(std::ffi::OsStr::new("tbz2"))
            {
                extract_tar_bz2(archive_path, dest_dir).await
            } else {
                Err(BuildError::DraftSourceFailed {
                    message: "Unsupported archive format: plain .bz2 files".to_string(),
                }
                .into())
            }
        }
        Some("xz" | "txz") => {
            if archive_path.to_string_lossy().ends_with(".tar.xz")
                || archive_path.extension() == Some(std::ffi::OsStr::new("txz"))
            {
                extract_tar_xz(archive_path, dest_dir).await
            } else {
                Err(BuildError::DraftSourceFailed {
                    message: "Unsupported archive format: plain .xz files".to_string(),
                }
                .into())
            }
        }
        _ => Err(BuildError::DraftSourceFailed {
            message: format!("Unsupported archive format: {}", archive_path.display()),
        }
        .into()),
    }
}

/// Extract a tar.gz archive
async fn extract_tar_gz(archive_path: PathBuf, dest_dir: PathBuf) -> Result<()> {
    extract_compressed_tar(archive_path, dest_dir, CompressionType::Gzip).await
}

/// Extract a tar.bz2 archive
async fn extract_tar_bz2(archive_path: PathBuf, dest_dir: PathBuf) -> Result<()> {
    extract_compressed_tar(archive_path, dest_dir, CompressionType::Bzip2).await
}

/// Extract a tar.xz archive
async fn extract_tar_xz(archive_path: PathBuf, dest_dir: PathBuf) -> Result<()> {
    extract_compressed_tar(archive_path, dest_dir, CompressionType::Xz).await
}

/// Compression types
enum CompressionType {
    Gzip,
    Bzip2,
    Xz,
}

/// Extract a compressed tar archive
async fn extract_compressed_tar(
    archive_path: PathBuf,
    dest_dir: PathBuf,
    compression: CompressionType,
) -> Result<()> {
    // Create a temporary file to decompress to
    let temp_file = tempfile::NamedTempFile::new().map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Failed to create temp file: {e}"),
    })?;
    let temp_path = temp_file.path().to_path_buf();

    // Decompress the archive
    {
        use tokio::fs::File;
        use tokio::io::AsyncWriteExt;

        let input_file =
            File::open(&archive_path)
                .await
                .map_err(|e| BuildError::DraftSourceFailed {
                    message: format!("Failed to open archive: {e}"),
                })?;

        let mut output_file =
            File::create(&temp_path)
                .await
                .map_err(|e| BuildError::DraftSourceFailed {
                    message: format!("Failed to create temp file: {e}"),
                })?;

        let reader = BufReader::new(input_file);
        let mut decoder: Box<dyn AsyncRead + Unpin> = match compression {
            CompressionType::Gzip => Box::new(GzipDecoder::new(reader)),
            CompressionType::Bzip2 => Box::new(BzDecoder::new(reader)),
            CompressionType::Xz => Box::new(XzDecoder::new(reader)),
        };

        tokio::io::copy(&mut decoder, &mut output_file)
            .await
            .map_err(|e| BuildError::DraftSourceFailed {
                message: format!("Failed to decompress archive: {e}"),
            })?;

        output_file
            .flush()
            .await
            .map_err(|e| BuildError::DraftSourceFailed {
                message: format!("Failed to flush temp file: {e}"),
            })?;
    }

    // Extract the decompressed tar file
    let temp_path_for_task = temp_path.clone();
    tokio::task::spawn_blocking(move || {
        use std::fs::File;

        let tar = File::open(&temp_path_for_task).map_err(|e| BuildError::DraftSourceFailed {
            message: format!("Failed to open decompressed file: {e}"),
        })?;
        let mut archive = TarArchive::new(tar);

        archive
            .unpack(&dest_dir)
            .map_err(|e| BuildError::DraftSourceFailed {
                message: format!("Failed to extract tar: {e}"),
            })?;

        Ok::<(), crate::Error>(())
    })
    .await
    .map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Task join error: {e}"),
    })??;

    Ok(())
}

/// Extract a plain tar archive
async fn extract_tar(archive_path: PathBuf, dest_dir: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        use std::fs::File;

        let tar = File::open(&archive_path).map_err(|e| BuildError::DraftSourceFailed {
            message: format!("Failed to open archive: {e}"),
        })?;
        let mut archive = TarArchive::new(tar);

        archive
            .unpack(&dest_dir)
            .map_err(|e| BuildError::DraftSourceFailed {
                message: format!("Failed to extract tar: {e}"),
            })?;

        Ok::<(), crate::Error>(())
    })
    .await
    .map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Task join error: {e}"),
    })?
}

/// Extract a zip archive
async fn extract_zip(archive_path: PathBuf, dest_dir: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        use std::fs::File;

        let file = File::open(&archive_path).map_err(|e| BuildError::DraftSourceFailed {
            message: format!("Failed to open archive: {e}"),
        })?;

        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| BuildError::DraftSourceFailed {
                message: format!("Failed to read zip archive: {e}"),
            })?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| BuildError::DraftSourceFailed {
                    message: format!("Failed to read zip entry: {e}"),
                })?;

            let outpath = match file.enclosed_name() {
                Some(path) => dest_dir.join(path),
                None => continue,
            };

            if file.name().ends_with('/') {
                std::fs::create_dir_all(&outpath).map_err(|e| BuildError::DraftSourceFailed {
                    message: format!("Failed to create directory: {e}"),
                })?;
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        std::fs::create_dir_all(p).map_err(|e| BuildError::DraftSourceFailed {
                            message: format!("Failed to create parent directory: {e}"),
                        })?;
                    }
                }
                let mut outfile =
                    File::create(&outpath).map_err(|e| BuildError::DraftSourceFailed {
                        message: format!("Failed to create file: {e}"),
                    })?;
                std::io::copy(&mut file, &mut outfile).map_err(|e| {
                    BuildError::DraftSourceFailed {
                        message: format!("Failed to extract file: {e}"),
                    }
                })?;
            }

            // Set permissions on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode)).ok();
                }
            }
        }

        Ok::<(), crate::Error>(())
    })
    .await
    .map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Task join error: {e}"),
    })?
}

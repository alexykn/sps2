//! Archive extraction utilities

use crate::Result;
use flate2::read::GzDecoder;
use sps2_errors::BuildError;
use std::fs::File;
use std::path::{Path, PathBuf};
use tar::Archive as TarArchive;
use tokio::task;

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
                Err(BuildError::DraftSourceFailed {
                    message: "TODO: .tar.bz2 extraction not yet implemented".to_string(),
                }
                .into())
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
                Err(BuildError::DraftSourceFailed {
                    message: "TODO: .tar.xz extraction not yet implemented".to_string(),
                }
                .into())
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
    task::spawn_blocking(move || {
        let tar_gz = File::open(&archive_path).map_err(|e| BuildError::DraftSourceFailed {
            message: format!("Failed to open archive: {e}"),
        })?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = TarArchive::new(tar);

        archive
            .unpack(&dest_dir)
            .map_err(|e| BuildError::DraftSourceFailed {
                message: format!("Failed to extract tar.gz: {e}"),
            })?;

        Ok(())
    })
    .await
    .map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Task join error: {e}"),
    })?
}

/// Extract a plain tar archive
async fn extract_tar(archive_path: PathBuf, dest_dir: PathBuf) -> Result<()> {
    task::spawn_blocking(move || {
        let tar = File::open(&archive_path).map_err(|e| BuildError::DraftSourceFailed {
            message: format!("Failed to open archive: {e}"),
        })?;
        let mut archive = TarArchive::new(tar);

        archive
            .unpack(&dest_dir)
            .map_err(|e| BuildError::DraftSourceFailed {
                message: format!("Failed to extract tar: {e}"),
            })?;

        Ok(())
    })
    .await
    .map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Task join error: {e}"),
    })?
}

/// Extract a zip archive
async fn extract_zip(archive_path: PathBuf, dest_dir: PathBuf) -> Result<()> {
    task::spawn_blocking(move || {
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

        Ok(())
    })
    .await
    .map_err(|e| BuildError::DraftSourceFailed {
        message: format!("Task join error: {e}"),
    })?
}

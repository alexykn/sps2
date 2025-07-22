//! Self-Update Functionality

use crate::OpsCtx;
use sps2_errors::{Error, OpsError};
use sps2_events::{AppEvent, EventEmitter, PackageEvent};
use std::path::Path;
use std::time::Instant;

/// Update sps2 to the latest version
///
/// # Errors
///
/// Returns an error if:
/// - Failed to check for latest version
/// - Failed to download or verify the new binary
/// - Failed to replace the current executable
pub async fn self_update(ctx: &OpsCtx, skip_verify: bool, force: bool) -> Result<String, Error> {
    let start = Instant::now();
    let current_version = env!("CARGO_PKG_VERSION");

    ctx.emit_event(AppEvent::Package(PackageEvent::SelfUpdateStarting));
    ctx.emit_event(AppEvent::Package(PackageEvent::SelfUpdateCheckingVersion {
        current_version: current_version.to_string(),
    }));

    // Check latest version from GitHub API
    let latest_version = get_latest_version(&ctx.net, &ctx.tx).await?;

    // Compare versions
    let current = sps2_types::Version::parse(current_version)?;
    let latest = sps2_types::Version::parse(&latest_version)?;

    if !force && latest <= current {
        ctx.emit_event(AppEvent::Package(PackageEvent::SelfUpdateAlreadyLatest {
            version: current_version.to_string(),
        }));
        return Ok(format!("Already on latest version: {current_version}"));
    }

    ctx.emit_event(AppEvent::Package(
        PackageEvent::SelfUpdateVersionAvailable {
            current_version: current_version.to_string(),
            latest_version: latest_version.clone(),
        },
    ));

    // Determine download URLs for ARM64 macOS
    let binary_url = format!(
        "https://github.com/sps-io/sps2/releases/download/v{latest_version}/sps2-{latest_version}-aarch64-apple-darwin"
    );
    let signature_url = format!("{binary_url}.minisig");

    ctx.emit_event(AppEvent::Package(PackageEvent::SelfUpdateDownloading {
        version: latest_version.clone(),
        url: binary_url.clone(),
    }));

    // Create temporary directory for download
    let temp_dir = tempfile::tempdir().map_err(|e| OpsError::SelfUpdateFailed {
        message: format!("Failed to create temp directory: {e}"),
    })?;

    let temp_binary = temp_dir.path().join("sps2-new");
    let temp_signature = temp_dir.path().join("sps2-new.minisig");

    // Download new binary
    sps2_net::download_file(&ctx.net, &binary_url, &temp_binary, None, &ctx.tx)
        .await
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to download binary: {e}"),
        })?;

    if !skip_verify {
        ctx.emit_event(AppEvent::Package(PackageEvent::SelfUpdateVerifying {
            version: latest_version.clone(),
        }));

        // Download signature
        sps2_net::download_file(&ctx.net, &signature_url, &temp_signature, None, &ctx.tx)
            .await
            .map_err(|e| OpsError::SelfUpdateFailed {
                message: format!("Failed to download signature: {e}"),
            })?;

        // Verify signature
        verify_binary_signature(&temp_binary, &temp_signature).await?;
    }

    ctx.emit_event(AppEvent::Package(PackageEvent::SelfUpdateInstalling {
        version: latest_version.clone(),
    }));

    // Replace current executable atomically
    replace_current_executable(&temp_binary).await?;

    let duration = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    ctx.emit_event(AppEvent::Package(PackageEvent::SelfUpdateCompleted {
        old_version: current_version.to_string(),
        new_version: latest_version.clone(),
        duration_ms: duration,
    }));

    Ok(format!(
        "Updated from {current_version} to {latest_version}"
    ))
}

/// Get latest version from GitHub releases API
async fn get_latest_version(
    net_client: &sps2_net::NetClient,
    tx: &sps2_events::EventSender,
) -> Result<String, Error> {
    let api_url = "https://api.github.com/repos/sps-io/sps2/releases/latest";

    let response_text = sps2_net::fetch_text(net_client, api_url, tx)
        .await
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to fetch release info: {e}"),
        })?;

    let release: serde_json::Value =
        serde_json::from_str(&response_text).map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to parse release JSON: {e}"),
        })?;

    let tag_name = release["tag_name"]
        .as_str()
        .ok_or_else(|| OpsError::SelfUpdateFailed {
            message: "Release JSON missing tag_name field".to_string(),
        })?;

    // Remove 'v' prefix if present
    let version = tag_name.strip_prefix('v').unwrap_or(tag_name);
    Ok(version.to_string())
}

/// Verify binary signature using minisign
async fn verify_binary_signature(binary_path: &Path, signature_path: &Path) -> Result<(), Error> {
    let binary_content =
        tokio::fs::read(binary_path)
            .await
            .map_err(|e| OpsError::SelfUpdateFailed {
                message: format!("Failed to read binary for verification: {e}"),
            })?;

    let signature_content = tokio::fs::read_to_string(signature_path)
        .await
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to read signature: {e}"),
        })?;

    // Parse signature
    let signature = minisign_verify::Signature::decode(&signature_content).map_err(|e| {
        OpsError::SelfUpdateFailed {
            message: format!("Failed to parse signature: {e}"),
        }
    })?;

    // Use the same release signing key as for packages
    // In production, this would be the same trusted key used for package verification
    let trusted_key = "RWSGOq2NVecA2UPNdBUZykp1MLhfMmkAK/SZSjK3bpq2q7I8LbSVVBDm";

    let public_key = minisign_verify::PublicKey::from_base64(trusted_key).map_err(|e| {
        OpsError::SelfUpdateFailed {
            message: format!("Failed to parse public key: {e}"),
        }
    })?;

    public_key
        .verify(&binary_content, &signature, false)
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Binary signature verification failed: {e}"),
        })?;

    Ok(())
}

/// Replace current executable atomically
async fn replace_current_executable(new_binary_path: &Path) -> Result<(), Error> {
    // Get current executable path
    let current_exe = std::env::current_exe().map_err(|e| OpsError::SelfUpdateFailed {
        message: format!("Failed to get current executable path: {e}"),
    })?;

    // Make new binary executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(new_binary_path)
            .await
            .map_err(|e| OpsError::SelfUpdateFailed {
                message: format!("Failed to get binary metadata: {e}"),
            })?
            .permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(new_binary_path, perms)
            .await
            .map_err(|e| OpsError::SelfUpdateFailed {
                message: format!("Failed to set binary permissions: {e}"),
            })?;
    }

    // Create backup of current executable
    let backup_path = current_exe.with_extension("backup");
    tokio::fs::copy(&current_exe, &backup_path)
        .await
        .map_err(|e| OpsError::SelfUpdateFailed {
            message: format!("Failed to create backup: {e}"),
        })?;

    // Atomic replacement using rename
    tokio::fs::rename(new_binary_path, &current_exe)
        .await
        .map_err(|e| {
            // Attempt to restore backup on failure
            if let Err(restore_err) = std::fs::rename(&backup_path, &current_exe) {
                OpsError::SelfUpdateFailed {
                    message: format!(
                        "Failed to replace executable: {e}. Also failed to restore backup: {restore_err}"
                    ),
                }
            } else {
                OpsError::SelfUpdateFailed {
                    message: format!("Failed to replace executable: {e}. Restored from backup."),
                }
            }
        })?;

    // Clean up backup on success
    let _ = tokio::fs::remove_file(backup_path).await;

    Ok(())
}

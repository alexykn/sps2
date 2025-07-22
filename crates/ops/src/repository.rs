//! Repository and Index Management Operations

use crate::{keys::KeyManager, OpsCtx};
use sps2_errors::{Error, OpsError};
use sps2_events::{AppEvent, EventEmitter, GeneralEvent, RepoEvent};
use std::time::Instant;

/// Sync repository index
///
/// # Errors
///
/// Returns an error if index synchronization fails.
pub async fn reposync(ctx: &OpsCtx) -> Result<String, Error> {
    let start = Instant::now();

    ctx.emit_event(AppEvent::Repo(RepoEvent::SyncStarting));

    // Check if index is stale (older than 7 days)
    let stale = ctx.index.is_stale(7);

    if !stale {
        let message = "Repository index is up to date".to_string();
        let _ = ctx.tx.send(AppEvent::Repo(RepoEvent::SyncCompleted {
            packages_updated: 0,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        }));
        return Ok(message);
    }

    // Repository URL (in real implementation, this would come from config)
    let base_url = "https://cdn.sps.io";
    let index_url = format!("{base_url}/index.json");
    let index_sig_url = format!("{base_url}/index.json.minisig");
    let keys_url = format!("{base_url}/keys.json");

    ctx.emit_event(AppEvent::Repo(RepoEvent::SyncStarted {
        url: base_url.to_string(),
    }));

    // 1. Download latest index.json and signature with `ETag` support
    let cached_etag = ctx.index.cache.load_etag().await.unwrap_or(None);

    let index_json =
        download_index_conditional(ctx, &index_url, cached_etag.as_deref(), start).await?;

    let index_signature = sps2_net::fetch_text(&ctx.net, &index_sig_url, &ctx.tx)
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to download index.json.minisig: {e}"),
        })?;

    // 2. Fetch and verify signing keys (with rotation support)
    let trusted_keys = fetch_and_verify_keys(&ctx.net, &keys_url, &ctx.tx)
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to verify signing keys: {e}"),
        })?;

    // 3. Verify signature of index.json
    verify_index_signature(&index_json, &index_signature, &trusted_keys).map_err(|e| {
        OpsError::RepoSyncFailed {
            message: format!("Index signature verification failed: {e}"),
        }
    })?;

    // Process and save the new index
    finalize_index_update(ctx, &index_json, start).await
}

/// Download index conditionally with `ETag` support
async fn download_index_conditional(
    ctx: &OpsCtx,
    index_url: &str,
    cached_etag: Option<&str>,
    start: Instant,
) -> Result<String, Error> {
    let response = sps2_net::fetch_text_conditional(&ctx.net, index_url, cached_etag, &ctx.tx)
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to download index.json: {e}"),
        })?;

    if let Some((content, new_etag)) = response {
        // Save new `ETag` if present
        if let Some(etag) = new_etag {
            if let Err(e) = ctx.index.cache.save_etag(&etag).await {
                // Log but don't fail the operation
                ctx.emit_warning(format!("Failed to save ETag: {e}"));
            }
        }
        Ok(content)
    } else {
        // Server returned 304 Not Modified - use cached content
        let _ = ctx.tx.send(AppEvent::Repo(RepoEvent::SyncCompleted {
            packages_updated: 0,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        }));
        Err(OpsError::RepoSyncFailed {
            message: "Repository index is unchanged (304 Not Modified)".to_string(),
        }
        .into())
    }
}

/// Process and save the new index
async fn finalize_index_update(
    ctx: &OpsCtx,
    index_json: &str,
    start: Instant,
) -> Result<String, Error> {
    // Parse the new index to count changes
    let old_package_count = ctx.index.index().map_or(0, |idx| idx.packages.len());

    // Load the new index into IndexManager
    let mut new_index_manager = ctx.index.clone();
    new_index_manager
        .load(Some(index_json))
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to parse new index: {e}"),
        })?;

    let new_package_count = new_index_manager
        .index()
        .map_or(0, |idx| idx.packages.len());
    let packages_updated = new_package_count.saturating_sub(old_package_count);

    // Save to cache
    new_index_manager
        .save_to_cache()
        .await
        .map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to save index cache: {e}"),
        })?;

    let message = if packages_updated > 0 {
        format!("Updated {packages_updated} packages from repository")
    } else {
        "Repository index updated (no new packages)".to_string()
    };

    ctx.emit_event(AppEvent::Repo(RepoEvent::SyncCompleted {
        packages_updated,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
    }));

    Ok(message)
}

/// Fetch and verify signing keys with rotation support
async fn fetch_and_verify_keys(
    net_client: &sps2_net::NetClient,
    keys_url: &str,
    tx: &sps2_events::EventSender,
) -> Result<Vec<String>, Error> {
    // Initialize key manager
    let mut key_manager = KeyManager::new("/opt/pm/keys");

    // Load existing trusted keys from disk
    key_manager.load_trusted_keys().await?;

    // Check if we have any trusted keys; if not, initialize with bootstrap
    if key_manager.get_trusted_keys().is_empty() {
        // Bootstrap key for initial trust - in production this would be:
        // 1. Compiled into the binary
        // 2. Distributed through secure channels
        // 3. Verified through multiple sources
        let bootstrap_key = "RWSGOq2NVecA2UPNdBUZykp1MLhfMmkAK/SZSjK3bpq2q7I8LbSVVBDm";

        let _ = tx.send(AppEvent::General(GeneralEvent::Warning {
            message: "Initializing with bootstrap key".to_string(),
            context: Some("First run - no trusted keys found".to_string()),
        }));

        key_manager
            .initialize_with_bootstrap(bootstrap_key)
            .await
            .map_err(|e| OpsError::RepoSyncFailed {
                message: format!("Failed to initialize bootstrap key: {e}"),
            })?;
    }

    // Fetch and verify keys from repository
    let trusted_keys = key_manager
        .fetch_and_verify_keys(net_client, keys_url, tx)
        .await?;

    let _ = tx.send(AppEvent::General(GeneralEvent::OperationCompleted {
        operation: "Key verification".to_string(),
        success: true,
    }));

    Ok(trusted_keys)
}

/// Verify minisign signature of index.json
fn verify_index_signature(
    index_content: &str,
    signature: &str,
    trusted_keys: &[String],
) -> Result<(), Error> {
    if index_content.is_empty() {
        return Err(OpsError::RepoSyncFailed {
            message: "Index content is empty".to_string(),
        }
        .into());
    }

    if signature.is_empty() {
        return Err(OpsError::RepoSyncFailed {
            message: "Signature is empty".to_string(),
        }
        .into());
    }

    if trusted_keys.is_empty() {
        return Err(OpsError::RepoSyncFailed {
            message: "No trusted keys available for verification".to_string(),
        }
        .into());
    }

    // Parse the minisign signature - expect format:
    // untrusted comment: <comment>
    // <base64-signature>
    let signature_lines: Vec<&str> = signature.lines().collect();
    if signature_lines.len() < 2 {
        return Err(OpsError::RepoSyncFailed {
            message: "Invalid minisign signature format - missing lines".to_string(),
        }
        .into());
    }

    if !signature_lines[0].starts_with("untrusted comment:") {
        return Err(OpsError::RepoSyncFailed {
            message: "Invalid minisign signature format - missing comment line".to_string(),
        }
        .into());
    }

    // Use the full signature content (not just the base64 part)
    let sig =
        minisign_verify::Signature::decode(signature).map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Failed to parse signature: {e}"),
        })?;

    // Try verification with each trusted key until one succeeds
    let mut verification_errors = Vec::new();

    for trusted_key in trusted_keys {
        match minisign_verify::PublicKey::from_base64(trusted_key) {
            Ok(public_key) => {
                // Try to verify with this key - the verify method handles key ID comparison internally
                match public_key.verify(index_content.as_bytes(), &sig, false) {
                    Ok(()) => {
                        // Signature verification successful
                        return Ok(());
                    }
                    Err(e) => {
                        verification_errors.push(format!("Key verification failed: {e}"));
                    }
                }
            }
            Err(e) => {
                verification_errors.push(format!("Invalid trusted key format: {e}"));
            }
        }
    }

    // If we get here, no key successfully verified the signature
    Err(OpsError::RepoSyncFailed {
        message: format!(
            "Index signature verification failed. Tried {} trusted keys. Errors: {}",
            trusted_keys.len(),
            verification_errors.join("; ")
        ),
    }
    .into())
}

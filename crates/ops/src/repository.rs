//! Repository and Index Management Operations

use crate::keys;
use crate::{keys::KeyManager, OpsCtx};
use dialoguer::{theme::ColorfulTheme, Confirm};
use sps2_config::{Config, RepositoryConfig};
use sps2_errors::{ConfigError, Error, OpsError, SigningError};
use sps2_events::{AppEvent, EventEmitter, GeneralEvent, RepoEvent};
use std::path::PathBuf;
use std::time::Instant;
/// Sync repository index
///
/// # Errors
///
/// Returns an error if index synchronization fails.
pub async fn reposync(ctx: &OpsCtx, yes: bool) -> Result<String, Error> {
    let start = Instant::now();

    ctx.emit(AppEvent::Repo(RepoEvent::SyncStarting));

    let mut candidates: Vec<&sps2_config::RepositoryConfig> = ctx.config.repos.get_all();
    candidates.sort_by_key(|r| r.priority);

    let base_url = match candidates.first() {
        Some(repo) => repo.url.clone(),
        None => {
            return Err(Error::Config(ConfigError::MissingField {
                field: "repositories".to_string(),
            }))
        }
    };

    let index_url = format!("{base_url}/index.json");
    let index_sig_url = format!("{base_url}/index.json.minisig");
    let keys_url = format!("{base_url}/keys.json");

    ctx.emit(AppEvent::Repo(RepoEvent::SyncStarted {
        url: base_url.to_string(),
    }));

    let cached_etag = ctx.index.cache.load_etag().await.unwrap_or(None);
    let index_json =
        download_index_conditional(ctx, &index_url, cached_etag.as_deref(), start).await?;

    let index_signature = sps2_net::fetch_text(&ctx.net, &index_sig_url, &ctx.tx).await?;

    let mut trusted_keys = fetch_and_verify_keys(ctx, &ctx.net, &keys_url, &ctx.tx).await?;

    if let Err(e) = sps2_signing::verify_minisign_bytes_with_keys(
        index_json.as_bytes(),
        &index_signature,
        &trusted_keys,
    ) {
        match e {
            SigningError::NoTrustedKeyFound { key_id } => {
                let repo_keys: keys::RepositoryKeys =
                    sps2_net::fetch_json(&ctx.net, &keys_url, &ctx.tx).await?;
                let key_to_trust = repo_keys.keys.iter().find(|k| k.key_id == key_id);

                if let Some(key) = key_to_trust {
                    let prompt = format!(
                        "The repository index is signed with a new key: {key_id}. Do you want to trust it?"
                    );
                    if yes
                        || Confirm::with_theme(&ColorfulTheme::default())
                            .with_prompt(prompt)
                            .interact()
                            .map_err(|e| {
                                Error::internal(format!("Failed to get user confirmation: {e}"))
                            })?
                    {
                        let mut key_manager =
                            KeyManager::new(PathBuf::from(sps2_config::fixed_paths::KEYS_DIR));
                        key_manager.load_trusted_keys().await?;
                        key_manager.import_key(key).await?;
                        trusted_keys = key_manager.get_trusted_keys();
                        // Re-verify
                        sps2_signing::verify_minisign_bytes_with_keys(
                            index_json.as_bytes(),
                            &index_signature,
                            &trusted_keys,
                        )?;
                    } else {
                        return Err(Error::Signing(SigningError::NoTrustedKeyFound { key_id }));
                    }
                } else {
                    return Err(Error::Signing(SigningError::NoTrustedKeyFound { key_id }));
                }
            }
            other_error => {
                return Err(OpsError::RepoSyncFailed {
                    message: format!("Index signature verification failed: {other_error}"),
                }
                .into());
            }
        }
    }

    // Enforce index freshness based on security policy
    if let Ok(parsed_index) = sps2_index::Index::from_json(&index_json) {
        let now = chrono::Utc::now();
        let age = now.signed_duration_since(parsed_index.metadata.timestamp);
        let max_days = i64::from(ctx.config.security.index_max_age_days);
        if age.num_days() > max_days {
            return Err(OpsError::RepoSyncFailed {
                message: format!(
                    "Repository index is stale: {} days old (max {} days)",
                    age.num_days(),
                    max_days
                ),
            }
            .into());
        }
    }

    finalize_index_update(ctx, &index_json, start).await
}

/// Add a new repository to the user's configuration.
///
/// # Errors
///
/// Returns an error if:
/// - The configuration file cannot be loaded or created
/// - The repository URL is invalid
/// - The configuration cannot be saved
pub async fn add_repo(_ctx: &OpsCtx, name: &str, url: &str) -> Result<String, Error> {
    let config_path = Config::default_path()?;
    let mut config = Config::load_or_default(&Some(config_path)).await?;

    if config.repos.extras.contains_key(name) {
        return Err(Error::Config(ConfigError::Invalid {
            message: format!("Repository '{name}' already exists."),
        }));
    }

    let new_repo = RepositoryConfig {
        url: url.to_string(),
        priority: 10,
        algorithm: "minisign".to_string(),
        key_ids: vec![],
    };
    config.repos.extras.insert(name.to_string(), new_repo);

    config.save().await?;

    Ok(format!("Repository '{name}' added successfully."))
}

/// Download index conditionally with `ETag` support
async fn download_index_conditional(
    ctx: &OpsCtx,
    index_url: &str,
    cached_etag: Option<&str>,
    start: Instant,
) -> Result<String, Error> {
    let response =
        sps2_net::fetch_text_conditional(&ctx.net, index_url, cached_etag, &ctx.tx).await?;

    if let Some((content, new_etag)) = response {
        if let Some(etag) = new_etag {
            if let Err(e) = ctx.index.cache.save_etag(&etag).await {
                ctx.emit_warning(format!("Failed to save ETag: {e}"));
            }
        }
        Ok(content)
    } else {
        let _ = ctx.tx.send(AppEvent::Repo(RepoEvent::SyncCompleted {
            packages_updated: 0,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            bytes_transferred: 0,
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
    let old_package_count = ctx.index.index().map_or(0, |idx| idx.packages.len());

    let mut new_index_manager = ctx.index.clone();
    new_index_manager.load(Some(index_json)).await?;

    let new_package_count = new_index_manager
        .index()
        .map_or(0, |idx| idx.packages.len());
    let packages_updated = new_package_count.saturating_sub(old_package_count);

    new_index_manager.save_to_cache().await?;

    let message = if packages_updated > 0 {
        format!("Updated {packages_updated} packages from repository")
    } else {
        "Repository index updated (no new packages)".to_string()
    };

    ctx.emit(AppEvent::Repo(RepoEvent::SyncCompleted {
        packages_updated,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        bytes_transferred: 0, // TODO: Track actual bytes transferred
    }));

    Ok(message)
}

/// Fetch and verify signing keys with rotation support
async fn fetch_and_verify_keys(
    _ctx: &OpsCtx,
    net_client: &sps2_net::NetClient,
    keys_url: &str,
    tx: &sps2_events::EventSender,
) -> Result<Vec<sps2_signing::PublicKeyRef>, Error> {
    let mut key_manager = KeyManager::new(PathBuf::from(sps2_config::fixed_paths::KEYS_DIR));

    key_manager.load_trusted_keys().await?;

    if key_manager.get_trusted_keys().is_empty() {
        let bootstrap_key = "RWSGOq2NVecA2UPNdBUZykp1MLhfMmkAK/SZSjK3bpq2q7I8LbSVVBDm";
        let _ = tx.send(AppEvent::General(GeneralEvent::Warning {
            message: "Initializing with bootstrap key".to_string(),
            context: Some("First run - no trusted keys found".to_string()),
        }));
        key_manager.initialize_with_bootstrap(bootstrap_key).await?;
    }

    let trusted_keys = key_manager
        .fetch_and_verify_keys(net_client, keys_url, tx)
        .await?;

    let _ = tx.send(AppEvent::General(GeneralEvent::OperationCompleted {
        operation: "Key verification".to_string(),
        success: true,
    }));

    Ok(trusted_keys)
}

//! Repository and Index Management Operations

use crate::keys;
use crate::{keys::KeyManager, OpsCtx};
use dialoguer::{theme::ColorfulTheme, Confirm};
use sps2_config::{Config, RepositoryConfig};
use sps2_errors::{ConfigError, Error, OpsError, SigningError};
use sps2_events::{AppEvent, EventEmitter, FailureContext, GeneralEvent, LifecycleEvent};
use std::path::PathBuf;
use std::time::Instant;
/// Sync repository index
///
/// # Errors
///
/// Returns an error if index synchronization fails.
///
/// # Panics
///
/// Panics if `base_url` is None after validation (should never happen).
pub async fn reposync(ctx: &OpsCtx, yes: bool) -> Result<String, Error> {
    let start = Instant::now();
    let _correlation = ctx.push_correlation("reposync");

    let Some(base_url) = get_base_url(ctx) else {
        let err = Error::Config(ConfigError::MissingField {
            field: "repositories".to_string(),
        });
        ctx.emit(AppEvent::Lifecycle(LifecycleEvent::repo_sync_failed(
            None,
            FailureContext::from_error(&err),
        )));
        return Err(err);
    };

    ctx.emit(AppEvent::Lifecycle(LifecycleEvent::repo_sync_started(
        Some(base_url.to_string()),
    )));

    let index_result = sync_and_verify_index(ctx, &base_url, start, yes).await;
    let index_json = match index_result {
        Ok(json) => json,
        Err(e) => {
            let failure = FailureContext::from_error(&e);
            ctx.emit(AppEvent::Lifecycle(LifecycleEvent::repo_sync_failed(
                Some(base_url.to_string()),
                failure,
            )));
            return Err(e);
        }
    };

    // Enforce index freshness based on security policy
    if let Ok(parsed_index) = sps2_index::Index::from_json(&index_json) {
        let now = chrono::Utc::now();
        let age = now.signed_duration_since(parsed_index.metadata.timestamp);
        let max_days = i64::from(ctx.config.security.index_max_age_days);
        if age.num_days() > max_days {
            let err = OpsError::RepoSyncFailed {
                message: format!(
                    "Repository index is stale: {} days old (max {} days)",
                    age.num_days(),
                    max_days
                ),
            }
            .into();
            ctx.emit(AppEvent::Lifecycle(LifecycleEvent::repo_sync_failed(
                Some(base_url.to_string()),
                FailureContext::from_error(&err),
            )));
            return Err(err);
        }
    }

    finalize_index_update(ctx, &index_json, start).await
}

fn get_base_url(ctx: &OpsCtx) -> Option<String> {
    let mut candidates: Vec<&sps2_config::RepositoryConfig> = ctx.config.repos.get_all();
    candidates.sort_by_key(|r| r.priority);
    candidates.first().map(|repo| repo.url.clone())
}

async fn sync_and_verify_index(
    ctx: &OpsCtx,
    base_url: &str,
    start: Instant,
    yes: bool,
) -> Result<String, Error> {
    let index_url = format!("{base_url}/index.json");
    let index_sig_url = format!("{base_url}/index.json.minisig");
    let keys_url = format!("{base_url}/keys.json");

    let cached_etag = ctx.index.cache.load_etag().await.unwrap_or(None);
    let index_json =
        download_index_conditional(ctx, &index_url, cached_etag.as_deref(), start).await?;
    let index_signature = sps2_net::fetch_text(&ctx.net, &index_sig_url, &ctx.tx).await?;
    let mut trusted_keys = fetch_and_verify_keys(ctx, &ctx.net, &keys_url, &ctx.tx).await?;

    if let Err(e) = sps2_net::verify_minisign_bytes_with_keys(
        index_json.as_bytes(),
        &index_signature,
        &trusted_keys,
    ) {
        handle_signature_verification_error(
            ctx,
            e,
            &keys_url,
            &index_json,
            &index_signature,
            yes,
            &mut trusted_keys,
        )
        .await?;
    }

    Ok(index_json)
}

async fn handle_signature_verification_error(
    ctx: &OpsCtx,
    e: SigningError,
    keys_url: &str,
    index_json: &str,
    index_signature: &str,
    yes: bool,
    trusted_keys: &mut Vec<sps2_net::PublicKeyRef>,
) -> Result<(), Error> {
    match e {
        SigningError::NoTrustedKeyFound { key_id } => {
            let repo_keys: keys::RepositoryKeys =
                sps2_net::fetch_json(&ctx.net, keys_url, &ctx.tx).await?;
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
                    *trusted_keys = key_manager.get_trusted_keys();
                    // Re-verify
                    sps2_net::verify_minisign_bytes_with_keys(
                        index_json.as_bytes(),
                        index_signature,
                        trusted_keys,
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

    Ok(())
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

/// List configured repositories from the user's configuration.
///
/// # Errors
///
/// Returns an error if the configuration file cannot be read.
pub async fn list_repos(_ctx: &OpsCtx) -> Result<String, Error> {
    let config_path = Config::default_path()?;
    let config = Config::load_or_default(&Some(config_path)).await?;

    let mut lines = Vec::new();

    if let Some(ref fast) = config.repos.fast {
        lines.push(format!(
            "fast:    {} (priority {})",
            fast.url, fast.priority
        ));
    }
    if let Some(ref slow) = config.repos.slow {
        lines.push(format!(
            "slow:    {} (priority {})",
            slow.url, slow.priority
        ));
    }
    if let Some(ref stable) = config.repos.stable {
        lines.push(format!(
            "stable:  {} (priority {})",
            stable.url, stable.priority
        ));
    }

    for (name, repo) in &config.repos.extras {
        lines.push(format!("{name}: {} (priority {})", repo.url, repo.priority));
    }

    if lines.is_empty() {
        Ok("No repositories configured.".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

/// Remove a repository by name. Supports standard names (fast/slow/stable) and extras.
///
/// # Errors
///
/// Returns an error if the configuration cannot be loaded or saved, or if the
/// named repository does not exist.
pub async fn remove_repo(_ctx: &OpsCtx, name: &str) -> Result<String, Error> {
    let config_path = Config::default_path()?;
    let mut config = Config::load_or_default(&Some(config_path)).await?;

    let mut removed = false;
    match name {
        "fast" => {
            if config.repos.fast.take().is_some() {
                removed = true;
            }
        }
        "slow" => {
            if config.repos.slow.take().is_some() {
                removed = true;
            }
        }
        "stable" => {
            if config.repos.stable.take().is_some() {
                removed = true;
            }
        }
        _ => {
            if config.repos.extras.remove(name).is_some() {
                removed = true;
            }
        }
    }

    if !removed {
        return Err(Error::Config(ConfigError::Invalid {
            message: format!("Repository '{name}' not found."),
        }));
    }

    config.save().await?;
    Ok(format!("Repository '{name}' removed successfully."))
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
        ctx.tx
            .emit(AppEvent::Lifecycle(LifecycleEvent::repo_sync_completed(
                0,
                u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                0,
            )));
        Ok("Repository index is unchanged (304 Not Modified)".to_string())
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

    ctx.emit(AppEvent::Lifecycle(LifecycleEvent::repo_sync_completed(
        packages_updated,
        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        0, // TODO: Track actual bytes transferred
    )));

    Ok(message)
}

/// Fetch and verify signing keys with rotation support
async fn fetch_and_verify_keys(
    _ctx: &OpsCtx,
    net_client: &sps2_net::NetClient,
    keys_url: &str,
    tx: &sps2_events::EventSender,
) -> Result<Vec<sps2_net::PublicKeyRef>, Error> {
    let mut key_manager = KeyManager::new(PathBuf::from(sps2_config::fixed_paths::KEYS_DIR));

    key_manager.load_trusted_keys().await?;

    if key_manager.get_trusted_keys().is_empty() {
        let bootstrap_key = "RWSGOq2NVecA2UPNdBUZykp1MLhfMmkAK/SZSjK3bpq2q7I8LbSVVBDm";
        tx.emit(AppEvent::General(GeneralEvent::Warning {
            message: "Initializing with bootstrap key".to_string(),
            context: Some("First run - no trusted keys found".to_string()),
        }));
        key_manager.initialize_with_bootstrap(bootstrap_key).await?;
    }

    let trusted_keys = key_manager
        .fetch_and_verify_keys(net_client, keys_url, tx)
        .await?;

    tx.emit(AppEvent::General(GeneralEvent::OperationCompleted {
        operation: "Key verification".to_string(),
        success: true,
    }));

    Ok(trusted_keys)
}

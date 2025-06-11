//! Key management utilities for signature verification

use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sps2_errors::{Error, OpsError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Key rotation information for verifying key changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRotation {
    /// Previous key ID that signed this rotation
    pub previous_key_id: String,
    /// New key to trust
    pub new_key: TrustedKey,
    /// Signature of the new key by the previous key
    pub rotation_signature: String,
    /// Timestamp of the rotation
    pub timestamp: i64,
}

/// A trusted public key with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedKey {
    /// Unique identifier for the key
    pub key_id: String,
    /// The minisign public key data
    pub public_key: String,
    /// Optional comment/description
    pub comment: Option<String>,
    /// Timestamp when key was first trusted
    pub trusted_since: i64,
    /// Optional expiration timestamp
    pub expires_at: Option<i64>,
}

/// Repository keys.json format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryKeys {
    /// Current active signing keys
    pub keys: Vec<TrustedKey>,
    /// Key rotation history
    pub rotations: Vec<KeyRotation>,
    /// Minimum signature age in seconds (to prevent replay attacks)
    pub max_signature_age: Option<u64>,
}

/// Key manager for handling trusted keys and verification
pub struct KeyManager {
    /// Path to keys directory (/opt/pm/keys/)
    keys_dir: PathBuf,
    /// Currently loaded trusted keys
    trusted_keys: HashMap<String, TrustedKey>,
    /// Bootstrap key for initial trust
    bootstrap_key: Option<TrustedKey>,
}

impl KeyManager {
    /// Create a new key manager
    pub fn new<P: AsRef<Path>>(keys_dir: P) -> Self {
        Self {
            keys_dir: keys_dir.as_ref().to_path_buf(),
            trusted_keys: HashMap::new(),
            bootstrap_key: None,
        }
    }

    /// Initialize the key manager with a bootstrap key
    ///
    /// # Errors
    ///
    /// Returns an error if the bootstrap key is invalid or directory creation fails.
    pub async fn initialize_with_bootstrap(&mut self, bootstrap_key: &str) -> Result<(), Error> {
        // Ensure keys directory exists
        fs::create_dir_all(&self.keys_dir).await?;

        // Parse and validate bootstrap key
        let _public_key =
            PublicKey::from_base64(bootstrap_key).map_err(|e| OpsError::RepoSyncFailed {
                message: format!("Invalid bootstrap key: {e}"),
            })?;

        // Generate a temporary key ID for bootstrapping
        // In practice, this would need to be derived from the actual key data
        // For now, we'll use a hash of the public key string
        let key_id = format!("bootstrap-{}", hex::encode(&bootstrap_key.as_bytes()[..8]));
        let bootstrap = TrustedKey {
            key_id: key_id.clone(),
            public_key: bootstrap_key.to_string(),
            comment: Some("Bootstrap key".to_string()),
            trusted_since: chrono::Utc::now().timestamp(),
            expires_at: None,
        };

        self.bootstrap_key = Some(bootstrap.clone());
        self.trusted_keys.insert(key_id, bootstrap);

        // Save bootstrap key to disk
        self.save_trusted_keys().await?;

        Ok(())
    }

    /// Load trusted keys from disk
    ///
    /// # Errors
    ///
    /// Returns an error if loading from disk fails.
    pub async fn load_trusted_keys(&mut self) -> Result<(), Error> {
        let keys_file = self.keys_dir.join("trusted_keys.json");

        if !keys_file.exists() {
            // No existing keys, start with empty set
            return Ok(());
        }

        let content = fs::read_to_string(&keys_file).await?;
        let keys: HashMap<String, TrustedKey> = serde_json::from_str(&content)
            .map_err(|e| Error::internal(format!("Failed to parse trusted keys: {e}")))?;

        self.trusted_keys = keys;
        Ok(())
    }

    /// Save trusted keys to disk
    ///
    /// # Errors
    ///
    /// Returns an error if saving to disk fails.
    pub async fn save_trusted_keys(&self) -> Result<(), Error> {
        let keys_file = self.keys_dir.join("trusted_keys.json");
        let content = serde_json::to_string_pretty(&self.trusted_keys)
            .map_err(|e| Error::internal(format!("Failed to serialize trusted keys: {e}")))?;
        fs::write(&keys_file, content).await?;
        Ok(())
    }

    /// Fetch and verify keys from repository
    ///
    /// # Errors
    ///
    /// Returns an error if fetching, parsing, or verifying keys fails.
    pub async fn fetch_and_verify_keys(
        &mut self,
        net_client: &sps2_net::NetClient,
        keys_url: &str,
        tx: &sps2_events::EventSender,
    ) -> Result<Vec<String>, Error> {
        // Fetch keys.json from repository
        let keys_content = sps2_net::fetch_text(net_client, keys_url, tx)
            .await
            .map_err(|e| OpsError::RepoSyncFailed {
                message: format!("Failed to fetch keys.json: {e}"),
            })?;

        // Parse repository keys
        let repo_keys: RepositoryKeys =
            serde_json::from_str(&keys_content).map_err(|e| OpsError::RepoSyncFailed {
                message: format!("Failed to parse repository keys: {e}"),
            })?;

        // Verify key rotations if any new keys are present
        self.verify_key_rotations(&repo_keys)?;

        // Update trusted keys with any new valid keys
        for key in &repo_keys.keys {
            // Check if key is already trusted
            if !self.trusted_keys.contains_key(&key.key_id) {
                // Verify this key was properly rotated in
                if self.is_key_rotation_valid(&repo_keys, &key.key_id) {
                    self.trusted_keys.insert(key.key_id.clone(), key.clone());
                }
            }
        }

        // Save updated keys
        self.save_trusted_keys().await?;

        // Return list of trusted key public key strings for verification
        Ok(self
            .trusted_keys
            .values()
            .map(|k| k.public_key.clone())
            .collect())
    }

    /// Verify signature against content using trusted keys
    ///
    /// # Errors
    ///
    /// Returns an error if signature verification fails.
    #[allow(dead_code)] // Method will be used in future implementations
    pub fn verify_signature(&self, content: &str, signature: &str) -> Result<(), Error> {
        // Parse the signature (expects full signature content)
        let sig = Signature::decode(signature).map_err(|e| OpsError::RepoSyncFailed {
            message: format!("Invalid signature format: {e}"),
        })?;

        // Try verification with each trusted key until one succeeds
        let mut verification_errors = Vec::new();
        let now = chrono::Utc::now().timestamp();

        for trusted_key in self.trusted_keys.values() {
            // Check if key is expired
            if let Some(expires_at) = trusted_key.expires_at {
                if now > expires_at {
                    verification_errors.push(format!("Key {} has expired", trusted_key.key_id));
                    continue;
                }
            }

            // Parse the public key and try verification
            match PublicKey::from_base64(&trusted_key.public_key) {
                Ok(public_key) => {
                    // Try to verify with this key - the verify method handles key ID comparison internally
                    match public_key.verify(content.as_bytes(), &sig, false) {
                        Ok(()) => {
                            // Signature verification successful
                            return Ok(());
                        }
                        Err(e) => {
                            verification_errors.push(format!("Key {}: {}", trusted_key.key_id, e));
                        }
                    }
                }
                Err(e) => {
                    verification_errors.push(format!(
                        "Invalid key format for {}: {}",
                        trusted_key.key_id, e
                    ));
                }
            }
        }

        // If we get here, no key successfully verified the signature
        Err(OpsError::RepoSyncFailed {
            message: format!(
                "Signature verification failed. Tried {} trusted keys. Errors: {}",
                self.trusted_keys.len(),
                verification_errors.join("; ")
            ),
        }
        .into())
    }

    /// Verify key rotations are valid
    fn verify_key_rotations(&self, repo_keys: &RepositoryKeys) -> Result<(), Error> {
        for rotation in &repo_keys.rotations {
            // Find the previous key that should have signed this rotation
            let previous_key = self
                .trusted_keys
                .get(&rotation.previous_key_id)
                .ok_or_else(|| OpsError::RepoSyncFailed {
                    message: format!(
                        "Key rotation references unknown previous key: {}",
                        rotation.previous_key_id
                    ),
                })?;

            // Verify the rotation signature
            let rotation_content = format!(
                "{}{}{}",
                rotation.new_key.key_id, rotation.new_key.public_key, rotation.timestamp
            );

            let previous_public_key =
                PublicKey::from_base64(&previous_key.public_key).map_err(|e| {
                    OpsError::RepoSyncFailed {
                        message: format!("Invalid previous key format: {e}"),
                    }
                })?;

            let rotation_sig = Signature::decode(&rotation.rotation_signature).map_err(|e| {
                OpsError::RepoSyncFailed {
                    message: format!("Invalid rotation signature format: {e}"),
                }
            })?;

            previous_public_key
                .verify(rotation_content.as_bytes(), &rotation_sig, false)
                .map_err(|e| OpsError::RepoSyncFailed {
                    message: format!("Key rotation signature verification failed: {e}"),
                })?;
        }

        Ok(())
    }

    /// Check if a key rotation is valid for a given key ID
    fn is_key_rotation_valid(&self, repo_keys: &RepositoryKeys, key_id: &str) -> bool {
        // If it's the bootstrap key, it's always valid
        if let Some(bootstrap) = &self.bootstrap_key {
            if bootstrap.key_id == key_id {
                return true;
            }
        }

        // Check if there's a valid rotation chain to this key
        for rotation in &repo_keys.rotations {
            if rotation.new_key.key_id == key_id
                && self.trusted_keys.contains_key(&rotation.previous_key_id)
            {
                return true;
            }
        }

        false
    }

    /// Get all trusted key public key strings
    pub fn get_trusted_keys(&self) -> Vec<String> {
        self.trusted_keys
            .values()
            .map(|k| k.public_key.clone())
            .collect()
    }
}

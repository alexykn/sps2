//! Key management utilities for signature verification

use base64::{engine::general_purpose, Engine as _};
use hex;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sps2_errors::Error;
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
    /// Unique identifier for the key (hex-encoded keynum)
    pub key_id: String,
    /// The minisign public key data (base64)
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
    /// Returns an error if:
    /// - The keys directory cannot be created
    /// - The bootstrap key string cannot be decoded
    /// - The public key cannot be parsed
    pub async fn initialize_with_bootstrap(
        &mut self,
        bootstrap_key_str: &str,
    ) -> Result<(), Error> {
        fs::create_dir_all(&self.keys_dir).await?;

        let decoded_pk = general_purpose::STANDARD
            .decode(bootstrap_key_str)
            .map_err(|e| {
                Error::Config(sps2_errors::ConfigError::Invalid {
                    message: e.to_string(),
                })
            })?;
        if decoded_pk.len() < 10 {
            return Err(Error::Config(sps2_errors::ConfigError::Invalid {
                message: "Invalid bootstrap key length".to_string(),
            }));
        }
        let key_id_bytes = &decoded_pk[2..10];
        let key_id = hex::encode(key_id_bytes);

        let bootstrap = TrustedKey {
            key_id: key_id.clone(),
            public_key: bootstrap_key_str.to_string(),
            comment: Some("Bootstrap key".to_string()),
            trusted_since: chrono::Utc::now().timestamp(),
            expires_at: None,
        };

        self.bootstrap_key = Some(bootstrap.clone());
        self.trusted_keys.insert(key_id, bootstrap);

        self.save_trusted_keys().await?;

        Ok(())
    }

    /// Load trusted keys from disk
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The trusted keys file cannot be read
    /// - The JSON content cannot be parsed
    pub async fn load_trusted_keys(&mut self) -> Result<(), Error> {
        let keys_file = self.keys_dir.join("trusted_keys.json");

        if !keys_file.exists() {
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
    /// Returns an error if:
    /// - The trusted keys cannot be serialized to JSON
    /// - The file cannot be written to disk
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
    /// Returns an error if:
    /// - The keys cannot be fetched from the repository
    /// - The keys content cannot be parsed as JSON
    /// - Signature verification fails
    pub async fn fetch_and_verify_keys(
        &mut self,
        net_client: &sps2_net::NetClient,
        keys_url: &str,
        tx: &sps2_events::EventSender,
    ) -> Result<Vec<sps2_signing::PublicKeyRef>, Error> {
        let keys_content = sps2_net::fetch_text(net_client, keys_url, tx).await?;

        let repo_keys: RepositoryKeys = serde_json::from_str(&keys_content)?;

        self.verify_key_rotations(&repo_keys)?;

        for key in &repo_keys.keys {
            if !self.trusted_keys.contains_key(&key.key_id)
                && self.is_key_rotation_valid(&repo_keys, &key.key_id)
            {
                self.trusted_keys.insert(key.key_id.clone(), key.clone());
            }
        }

        self.save_trusted_keys().await?;

        Ok(self
            .trusted_keys
            .values()
            .map(|k| sps2_signing::PublicKeyRef {
                id: k.key_id.clone(),
                algo: sps2_signing::Algorithm::Minisign,
                data: k.public_key.clone(),
            })
            .collect())
    }

    /// Verify signature against content using trusted keys
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The signature cannot be decoded
    /// - None of the trusted keys can verify the signature
    /// - The signature has expired
    #[allow(dead_code)]
    pub fn verify_signature(&self, content: &str, signature: &str) -> Result<(), Error> {
        let sig = Signature::decode(signature)?;

        let mut verification_errors = Vec::new();
        let now = chrono::Utc::now().timestamp();

        for trusted_key in self.trusted_keys.values() {
            if let Some(expires_at) = trusted_key.expires_at {
                if now > expires_at {
                    verification_errors.push(format!("Key {} has expired", trusted_key.key_id));
                    continue;
                }
            }

            match PublicKey::from_base64(&trusted_key.public_key) {
                Ok(public_key) => match public_key.verify(content.as_bytes(), &sig, false) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        verification_errors.push(format!("Key {}: {}", trusted_key.key_id, e));
                    }
                },
                Err(e) => {
                    verification_errors.push(format!(
                        "Invalid key format for {}: {}",
                        trusted_key.key_id, e
                    ));
                }
            }
        }

        Err(Error::Signing(
            sps2_errors::SigningError::VerificationFailed {
                reason: format!(
                    "Signature verification failed. Tried {} trusted keys. Errors: {}",
                    self.trusted_keys.len(),
                    verification_errors.join("; ")
                ),
            },
        ))
    }

    /// Verify key rotations are valid
    fn verify_key_rotations(&self, repo_keys: &RepositoryKeys) -> Result<(), Error> {
        for rotation in &repo_keys.rotations {
            let previous_key = self
                .trusted_keys
                .get(&rotation.previous_key_id)
                .ok_or_else(|| {
                    Error::Signing(sps2_errors::SigningError::NoTrustedKeyFound {
                        key_id: rotation.previous_key_id.clone(),
                    })
                })?;

            let rotation_content = format!(
                "{}{}{}",
                rotation.new_key.key_id, rotation.new_key.public_key, rotation.timestamp
            );

            let previous_public_key = PublicKey::from_base64(&previous_key.public_key)?;

            let rotation_sig = Signature::decode(&rotation.rotation_signature)?;

            previous_public_key.verify(rotation_content.as_bytes(), &rotation_sig, false)?;
        }

        Ok(())
    }

    /// Check if a key rotation is valid for a given key ID
    fn is_key_rotation_valid(&self, repo_keys: &RepositoryKeys, key_id: &str) -> bool {
        if let Some(bootstrap) = &self.bootstrap_key {
            if bootstrap.key_id == key_id {
                return true;
            }
        }

        for rotation in &repo_keys.rotations {
            if rotation.new_key.key_id == key_id
                && self.trusted_keys.contains_key(&rotation.previous_key_id)
            {
                return true;
            }
        }

        false
    }

    /// Get all trusted keys
    #[must_use]
    pub fn get_trusted_keys(&self) -> Vec<sps2_signing::PublicKeyRef> {
        self.trusted_keys
            .values()
            .map(|k| sps2_signing::PublicKeyRef {
                id: k.key_id.clone(),
                algo: sps2_signing::Algorithm::Minisign,
                data: k.public_key.clone(),
            })
            .collect()
    }

    /// Import a new key into the trusted set
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The trusted keys cannot be saved to disk
    pub async fn import_key(&mut self, key: &TrustedKey) -> Result<(), Error> {
        if self.trusted_keys.contains_key(&key.key_id) {
            return Ok(()); // Key already trusted
        }

        self.trusted_keys.insert(key.key_id.clone(), key.clone());
        self.save_trusted_keys().await
    }
}

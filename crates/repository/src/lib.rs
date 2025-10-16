#![warn(mismatched_lifetime_syntaxes)]
#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

use base64::Engine as _;
use chrono::Utc;
use regex::Regex;
use sps2_errors::{Error, StorageError};
use sps2_hash::Hash;
use sps2_index::{DependencyInfo, Index, VersionEntry};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone)]
pub struct PackageArtifact {
    pub name: String,
    pub version: String,
    pub revision: u32,
    pub arch: String,
    pub blake3: String,
    pub filename: String,
}

#[async_trait::async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put_object(&self, key: &str, bytes: &[u8]) -> Result<(), Error>;
    async fn get_object(&self, key: &str) -> Result<Vec<u8>, Error>;
    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, Error>;
}

/// Local filesystem-backed object store for development
#[derive(Debug, Clone)]
pub struct LocalStore {
    base: PathBuf,
}

impl LocalStore {
    #[must_use]
    pub fn new<P: Into<PathBuf>>(base: P) -> Self {
        Self { base: base.into() }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.base.join(key)
    }
}

#[async_trait::async_trait]
impl ObjectStore for LocalStore {
    async fn put_object(&self, key: &str, bytes: &[u8]) -> Result<(), Error> {
        let path = self.path_for(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&path, bytes).await?;
        Ok(())
    }

    async fn get_object(&self, key: &str) -> Result<Vec<u8>, Error> {
        let path = self.path_for(key);
        Ok(fs::read(&path).await?)
    }

    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, Error> {
        let mut results = Vec::new();
        let dir = self.base.join(prefix);
        let dir = if dir.is_dir() { dir } else { self.base.clone() };
        let mut rd = fs::read_dir(&dir)
            .await
            .map_err(|e| StorageError::IoError {
                message: e.to_string(),
            })?;
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    results.push(name.to_string());
                }
            }
        }
        Ok(results)
    }
}

/// Publisher builds and signs an index from objects in a store
#[derive(Debug, Clone)]
pub struct Publisher<S: ObjectStore> {
    pub store: S,
    pub base_url: String,
}

impl<S: ObjectStore> Publisher<S> {
    #[must_use]
    pub fn new(store: S, base_url: String) -> Self {
        Self { store, base_url }
    }

    /// Scan a directory for `.sp` files and return artifacts.
    ///
    /// # Errors
    ///
    /// Returns an error if directory entries cannot be read, or if hashing any
    /// matched package file fails.
    pub async fn scan_packages_local_dir(&self, dir: &Path) -> Result<Vec<PackageArtifact>, Error> {
        let mut artifacts = Vec::new();
        let mut rd = fs::read_dir(dir).await?;
        let re = Regex::new(r"^(.+?)-([^-]+)-(\d+)\.([^.]+)\.sp$")
            .map_err(|e| Error::internal(e.to_string()))?;
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("sp") {
                continue;
            }
            let filename = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| Error::internal("invalid filename"))?
                .to_string();
            if let Some(caps) = re.captures(&filename) {
                // Be defensive and skip if any capture group is missing
                let Some(g1) = caps.get(1) else { continue };
                let Some(g2) = caps.get(2) else { continue };
                let Some(g3) = caps.get(3) else { continue };
                let Some(g4) = caps.get(4) else { continue };

                let name = g1.as_str().to_string();
                let version = g2.as_str().to_string();
                let revision: u32 = g3.as_str().parse().unwrap_or(1);
                let arch = g4.as_str().to_string();

                // Compute BLAKE3 hash
                let hash = Hash::blake3_hash_file(&path).await?.to_hex();

                artifacts.push(PackageArtifact {
                    name,
                    version,
                    revision,
                    arch,
                    blake3: hash,
                    filename,
                });
            }
        }
        Ok(artifacts)
    }

    /// Build an Index from artifacts
    #[must_use]
    pub fn build_index(&self, artifacts: &[PackageArtifact]) -> Index {
        let mut index = Index::new();
        for a in artifacts {
            let entry = VersionEntry {
                revision: a.revision,
                arch: a.arch.clone(),
                blake3: a.blake3.clone(),
                download_url: format!("{}/{}", self.base_url.trim_end_matches('/'), a.filename),
                minisig_url: format!(
                    "{}/{}.minisig",
                    self.base_url.trim_end_matches('/'),
                    a.filename
                ),
                dependencies: DependencyInfo::default(),
                sbom: None,
                description: None,
                homepage: None,
                license: None,
            };
            index.add_version(a.name.clone(), a.version.clone(), entry);
        }
        index
    }

    /// Serialize and sign index, then publish `index.json` and `index.json.minisig` to store.
    ///
    /// # Errors
    ///
    /// Returns an error if index serialization fails, minisign signing fails,
    /// or writing to the object store fails.
    pub async fn publish_index(
        &self,
        index: &Index,
        secret_key_path: &Path,
        passphrase_or_keychain: Option<&str>,
    ) -> Result<(), Error> {
        let json = index.to_json()?;
        let sig = sps2_net::signing::minisign_sign_bytes(
            json.as_bytes(),
            secret_key_path,
            passphrase_or_keychain,
            Some("sps2 repository index"),
            Some("index.json"),
        )?;

        self.store.put_object("index.json", json.as_bytes()).await?;
        self.store
            .put_object("index.json.minisig", sig.as_bytes())
            .await?;
        Ok(())
    }
}

/// Keys.json model and helpers
pub mod keys {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TrustedKey {
        pub key_id: String,
        pub public_key: String, // base64
        pub comment: Option<String>,
        pub trusted_since: i64,
        pub expires_at: Option<i64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct KeyRotation {
        pub previous_key_id: String,
        pub new_key: TrustedKey,
        pub rotation_signature: String,
        pub timestamp: i64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RepositoryKeys {
        pub keys: Vec<TrustedKey>,
        pub rotations: Vec<KeyRotation>,
        #[serde(default)]
        pub max_signature_age: Option<u64>,
    }

    /// Derive minisign `key_id` from public key base64 (bytes[2..10]).
    ///
    /// # Errors
    ///
    /// Returns an error if the base64 payload cannot be decoded or is too short.
    pub fn key_id_from_public_base64(b64: &str) -> Result<String, Error> {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| Error::internal(format!("invalid minisign public key: {e}")))?;
        if decoded.len() < 10 {
            return Err(Error::internal("minisign public key too short"));
        }
        Ok(hex::encode(&decoded[2..10]))
    }

    /// Extract base64 from a minisign public key box or return input if it's already base64
    #[must_use]
    pub fn extract_base64(pk_input: &str) -> String {
        let trimmed = pk_input.trim();
        if trimmed.lines().count() <= 1 && !trimmed.contains(' ') {
            return trimmed.to_string();
        }
        // Parse box: skip first line, take next non-empty line
        let mut lines = trimmed.lines();
        let _ = lines.next();
        for line in lines {
            let l = line.trim();
            if !l.is_empty() {
                return l.to_string();
            }
        }
        trimmed.to_string()
    }

    /// Write `keys.json` to the repository directory.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or writing to disk fails.
    pub async fn write_keys_json(dir: &Path, repo_keys: &RepositoryKeys) -> Result<(), Error> {
        let content = serde_json::to_string_pretty(repo_keys)
            .map_err(|e| Error::internal(format!("serialize keys.json: {e}")))?;
        let path = dir.join("keys.json");
        fs::write(path, content).await?;
        Ok(())
    }

    /// Create a `RepositoryKeys` with a single trusted key and no rotations.
    ///
    /// # Errors
    ///
    /// Returns an error if deriving the minisign key id from the provided
    /// base64 public key fails.
    pub fn make_single_key(
        pk_base64: String,
        comment: Option<String>,
    ) -> Result<RepositoryKeys, Error> {
        let key_id = key_id_from_public_base64(&pk_base64)?;
        let trusted = TrustedKey {
            key_id,
            public_key: pk_base64,
            comment,
            trusted_since: Utc::now().timestamp(),
            expires_at: None,
        };
        Ok(RepositoryKeys {
            keys: vec![trusted],
            rotations: Vec::new(),
            max_signature_age: None,
        })
    }
}

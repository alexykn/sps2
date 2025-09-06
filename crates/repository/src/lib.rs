#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

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

    /// Scan a directory for .sp files and return artifacts
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
                let name = caps.get(1).unwrap().as_str().to_string();
                let version = caps.get(2).unwrap().as_str().to_string();
                let revision: u32 = caps.get(3).unwrap().as_str().parse().unwrap_or(1);
                let arch = caps.get(4).unwrap().as_str().to_string();

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

    /// Serialize and sign index, then publish `index.json` and `index.json.minisig` to store
    pub async fn publish_index(
        &self,
        index: &Index,
        secret_key_path: &Path,
        passphrase_or_keychain: Option<&str>,
    ) -> Result<(), Error> {
        let json = index.to_json()?;
        let sig = sps2_signing::minisign_sign_bytes(
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

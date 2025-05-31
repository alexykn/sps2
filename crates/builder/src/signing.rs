//! Package signing with Minisign

use minisign::{sign, KeyPair, PublicKey, SecretKey, SecretKeyBox, SignatureBox};
use sps2_errors::{BuildError, Error};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Package signing configuration
#[derive(Debug, Clone)]
pub struct SigningConfig {
    /// Path to the private key file
    pub private_key_path: Option<PathBuf>,
    /// Private key password
    pub key_password: Option<String>,
    /// Trusted comment to include in signature
    pub trusted_comment: Option<String>,
    /// Enable signing (false for testing/development)
    pub enabled: bool,
}

impl Default for SigningConfig {
    fn default() -> Self {
        Self {
            private_key_path: None,
            key_password: None,
            trusted_comment: Some("sps2 package signature".to_string()),
            enabled: false, // Disabled by default for development
        }
    }
}

impl SigningConfig {
    /// Create config with signing enabled
    #[must_use]
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// Set private key path
    #[must_use]
    pub fn with_private_key<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.private_key_path = Some(path.into());
        self
    }

    /// Set key password
    #[must_use]
    pub fn with_password<S: Into<String>>(mut self, password: S) -> Self {
        self.key_password = Some(password.into());
        self
    }

    /// Set trusted comment
    #[must_use]
    pub fn with_comment<S: Into<String>>(mut self, comment: S) -> Self {
        self.trusted_comment = Some(comment.into());
        self
    }
}

/// Package signer using Minisign
pub struct PackageSigner {
    config: SigningConfig,
}

impl PackageSigner {
    /// Create new package signer
    #[must_use]
    pub fn new(config: SigningConfig) -> Self {
        Self { config }
    }

    /// Sign a package file, creating a detached .minisig signature
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The private key path is not configured or doesn't exist
    /// - The package file doesn't exist
    /// - Key decryption or signature creation fails
    /// - Writing the signature file fails
    pub async fn sign_package(&self, package_path: &Path) -> Result<Option<PathBuf>, Error> {
        if !self.config.enabled {
            return Ok(None);
        }

        let private_key_path =
            self.config
                .private_key_path
                .as_ref()
                .ok_or_else(|| BuildError::SigningError {
                    message: "No private key path configured".to_string(),
                })?;

        if !private_key_path.exists() {
            return Err(BuildError::SigningError {
                message: format!("Private key file not found: {}", private_key_path.display()),
            }
            .into());
        }

        if !package_path.exists() {
            return Err(BuildError::SigningError {
                message: format!("Package file not found: {}", package_path.display()),
            }
            .into());
        }

        // Read the private key
        let key_data = fs::read(private_key_path)
            .await
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to read private key: {e}"),
            })?;

        // Parse secret key from file
        let sk_box_str = String::from_utf8(key_data).map_err(|e| BuildError::SigningError {
            message: format!("Invalid UTF-8 in private key file: {e}"),
        })?;

        let sk_box =
            SecretKeyBox::from_string(&sk_box_str).map_err(|e| BuildError::SigningError {
                message: format!("Failed to parse private key: {e}"),
            })?;

        let secret_key = sk_box
            .into_secret_key(self.config.key_password.clone())
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to decrypt private key: {e}"),
            })?;

        // Read the package file to sign
        let package_data = fs::read(package_path)
            .await
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to read package file: {e}"),
            })?;

        // Create signature
        let trusted_comment = self
            .config
            .trusted_comment
            .as_deref()
            .unwrap_or("sps2 package signature");
        let untrusted_comment = format!(
            "signature from sps2 for {}",
            package_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );

        let package_reader = Cursor::new(&package_data);
        let signature = sign(
            None, // No additional public key validation
            &secret_key,
            package_reader,
            Some(trusted_comment),
            Some(&untrusted_comment),
        )
        .map_err(|e| BuildError::SigningError {
            message: format!("Failed to create signature: {e}"),
        })?;

        // Write signature to .minisig file
        let sig_path = package_path.with_extension("sp.minisig");
        fs::write(&sig_path, signature.into_string())
            .await
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to write signature file: {e}"),
            })?;

        Ok(Some(sig_path))
    }

    /// Verify a package signature (for testing)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The package or signature files cannot be read
    /// - The signature cannot be parsed
    pub async fn verify_package(
        &self,
        package_path: &Path,
        public_key: &PublicKey,
    ) -> Result<bool, Error> {
        let sig_path = package_path.with_extension("sp.minisig");

        if !sig_path.exists() {
            return Ok(false);
        }

        // Read package data and signature
        let package_data = fs::read(package_path)
            .await
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to read package file: {e}"),
            })?;

        let sig_data =
            fs::read_to_string(&sig_path)
                .await
                .map_err(|e| BuildError::SigningError {
                    message: format!("Failed to read signature file: {e}"),
                })?;

        // Parse and verify signature
        let signature_box =
            SignatureBox::from_string(&sig_data).map_err(|e| BuildError::SigningError {
                message: format!("Failed to parse signature: {e}"),
            })?;

        let package_reader = Cursor::new(&package_data);
        let is_valid = minisign::verify(
            public_key,
            &signature_box,
            package_reader,
            true,
            false,
            false,
        )
        .is_ok();

        Ok(is_valid)
    }

    /// Generate a new key pair for signing (development/testing only)
    ///
    /// # Errors
    ///
    /// Returns an error if key pair generation fails.
    pub fn generate_keypair() -> Result<(SecretKey, PublicKey), Error> {
        // Use unencrypted keypair for testing to avoid interactive prompts
        let KeyPair { pk, sk } =
            KeyPair::generate_unencrypted_keypair().map_err(|e| BuildError::SigningError {
                message: format!("Failed to generate key pair: {e}"),
            })?;

        Ok((sk, pk))
    }

    /// Generate a new key pair with encryption for signing
    ///
    /// # Errors
    ///
    /// Returns an error if key pair generation fails.
    pub fn generate_encrypted_keypair(
        password: Option<&str>,
    ) -> Result<(SecretKeyBox, PublicKey), Error> {
        let KeyPair { pk, sk } =
            KeyPair::generate_unencrypted_keypair().map_err(|e| BuildError::SigningError {
                message: format!("Failed to generate key pair: {e}"),
            })?;

        let sk_box = sk.to_box(password).map_err(|e| BuildError::SigningError {
            message: format!("Failed to encrypt secret key: {e}"),
        })?;

        Ok((sk_box, pk))
    }

    /// Save a secret key to file
    ///
    /// # Errors
    ///
    /// Returns an error if key serialization or file writing fails.
    pub async fn save_secret_key(
        secret_key: &SecretKey,
        path: &Path,
        password: Option<&str>,
    ) -> Result<(), Error> {
        let sk_box = secret_key
            .to_box(password)
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to serialize secret key: {e}"),
            })?;

        fs::write(path, sk_box.to_string())
            .await
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to write secret key: {e}"),
            })?;

        Ok(())
    }

    /// Save a secret key box to file
    ///
    /// # Errors
    ///
    /// Returns an error if file writing fails.
    pub async fn save_secret_key_box(sk_box: &SecretKeyBox, path: &Path) -> Result<(), Error> {
        fs::write(path, sk_box.to_string())
            .await
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to write secret key: {e}"),
            })?;

        Ok(())
    }

    /// Save a public key to file
    ///
    /// # Errors
    ///
    /// Returns an error if key serialization or file writing fails.
    pub async fn save_public_key(public_key: &PublicKey, path: &Path) -> Result<(), Error> {
        let pk_box = public_key.to_box().map_err(|e| BuildError::SigningError {
            message: format!("Failed to serialize public key: {e}"),
        })?;

        fs::write(path, pk_box.to_string())
            .await
            .map_err(|e| BuildError::SigningError {
                message: format!("Failed to write public key: {e}"),
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_keypair_generation() {
        let result = PackageSigner::generate_keypair();
        assert!(result.is_ok());
        let (_secret_key, public_key) = result.unwrap();

        // Basic sanity check that generation worked
        // We can at least verify the public key can be serialized
        assert!(public_key.to_box().is_ok());
    }

    #[tokio::test]
    async fn test_public_key_serialization() {
        let temp = tempdir().unwrap();
        let (_secret_key, public_key) = PackageSigner::generate_keypair().unwrap();
        let public_path = temp.path().join("public.pub");

        // Test public key serialization (which doesn't involve passwords)
        PackageSigner::save_public_key(&public_key, &public_path)
            .await
            .unwrap();

        // Verify file exists and can be read
        assert!(public_path.exists());
        let key_data = fs::read_to_string(&public_path).await.unwrap();
        assert!(!key_data.is_empty());
        assert!(key_data.contains("minisign public key"));
    }

    #[tokio::test]
    async fn test_package_signing_disabled() {
        let temp = tempdir().unwrap();
        let package_path = temp.path().join("test.sp");
        fs::write(&package_path, b"test package data")
            .await
            .unwrap();

        let config = SigningConfig::default(); // disabled by default
        let signer = PackageSigner::new(config);

        let result = signer.sign_package(&package_path).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_signing_disabled_by_default() {
        let temp = tempdir().unwrap();
        let package_path = temp.path().join("test.sp");

        // Create test package
        fs::write(&package_path, b"test package data")
            .await
            .unwrap();

        // Test that signing is disabled by default
        let config = SigningConfig::default();
        assert!(!config.enabled);

        let signer = PackageSigner::new(config);
        let result = signer.sign_package(&package_path).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_signing_config_builder() {
        // Test the config builder pattern
        let config = SigningConfig::enabled()
            .with_private_key("/path/to/key")
            .with_password("test")
            .with_comment("custom comment");

        assert!(config.enabled);
        assert_eq!(config.private_key_path, Some(PathBuf::from("/path/to/key")));
        assert_eq!(config.key_password, Some("test".to_string()));
        assert_eq!(config.trusted_comment, Some("custom comment".to_string()));
    }

    #[tokio::test]
    async fn test_signing_error_cases() {
        let temp = tempdir().unwrap();
        let package_path = temp.path().join("test.sp");
        let nonexistent_key = temp.path().join("nonexistent.key");

        fs::write(&package_path, b"test package data")
            .await
            .unwrap();

        // Test missing private key path
        let config = SigningConfig::enabled();
        let signer = PackageSigner::new(config);
        let result = signer.sign_package(&package_path).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No private key path"));

        // Test nonexistent private key file
        let config = SigningConfig::enabled().with_private_key(&nonexistent_key);
        let signer = PackageSigner::new(config);
        let result = signer.sign_package(&package_path).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Private key file not found"));

        // Test nonexistent private key file
        let config = SigningConfig::enabled().with_private_key(&nonexistent_key);
        let signer = PackageSigner::new(config);
        let result = signer.sign_package(&package_path).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Private key file not found"));

        // Test nonexistent package file
        // For this test, we skip the actual key operations due to macOS issues
        // and just test the early validation
        let fake_key_path = temp.path().join("fake.key");
        fs::write(&fake_key_path, "fake key content").await.unwrap();

        let config = SigningConfig::enabled().with_private_key(&fake_key_path);
        let signer = PackageSigner::new(config);
        let nonexistent_package = temp.path().join("nonexistent.sp");
        let result = signer.sign_package(&nonexistent_package).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Package file not found"));
    }
}

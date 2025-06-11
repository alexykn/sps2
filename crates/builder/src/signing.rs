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

#![deny(clippy::pedantic, unsafe_code)]

use clap::{Parser, Subcommand};
use sps2_errors::Error;
use sps2_repository::keys as repo_keys;
use sps2_repository::{LocalStore, Publisher};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "sbs")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "SPS2 build/publish tooling", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Publish a single package (.sp) into the repository and update the index
    Publish {
        /// Path to the .sp file
        package: PathBuf,
        /// Repository directory path (local filesystem)
        #[arg(long, value_name = "DIR")]
        repo_dir: PathBuf,
        /// Base URL for download links in index (e.g., <http://localhost:8680>)
        #[arg(long, value_name = "URL")]
        base_url: String,
        /// Minisign secret key path
        #[arg(long, value_name = "PATH")]
        key: PathBuf,
        /// Optional passphrase or keychain string for minisign
        #[arg(long)]
        pass: Option<String>,
    },

    /// Rescan repo directory and rebuild+sign index
    UpdateIndices {
        /// Repository directory path (local filesystem)
        #[arg(long, value_name = "DIR")]
        repo_dir: PathBuf,
        /// Base URL for download links in index
        #[arg(long, value_name = "URL")]
        base_url: String,
        /// Minisign secret key path
        #[arg(long, value_name = "PATH")]
        key: PathBuf,
        /// Optional passphrase or keychain string for minisign
        #[arg(long)]
        pass: Option<String>,
    },

    /// Initialize a repository with keys.json
    RepoInit {
        /// Repository directory path
        #[arg(long, value_name = "DIR")]
        repo_dir: PathBuf,
        /// Use an existing Minisign public key file (.pub). If not provided, you can --generate.
        #[arg(long, value_name = "PUBFILE")]
        pubkey: Option<PathBuf>,
        /// Generate a new unencrypted key pair for testing
        #[arg(long, conflicts_with = "pubkey")]
        generate: bool,
        /// Output path for generated secret key (required with --generate)
        #[arg(long, requires = "generate", value_name = "PATH")]
        out_secret: Option<PathBuf>,
        /// Output path for generated public key (required with --generate)
        #[arg(long, requires = "generate", value_name = "PATH")]
        out_public: Option<PathBuf>,
        /// Optional comment to embed into keys.json
        #[arg(long)]
        comment: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Commands::Publish {
            package,
            repo_dir,
            base_url,
            key,
            pass,
        } => publish_one(package, repo_dir, base_url, key, pass).await?,
        Commands::UpdateIndices {
            repo_dir,
            base_url,
            key,
            pass,
        } => update_indices(repo_dir, base_url, key, pass).await?,
        Commands::RepoInit {
            repo_dir,
            pubkey,
            generate,
            out_secret,
            out_public,
            comment,
        } => {
            repo_init(repo_dir, pubkey, generate, out_secret, out_public, comment).await?;
        }
    }
    Ok(())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

async fn publish_one(
    package: PathBuf,
    repo_dir: PathBuf,
    base_url: String,
    key: PathBuf,
    pass: Option<String>,
) -> Result<(), Error> {
    // Copy .sp into repo dir
    let filename = package
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::internal("invalid package filename"))?
        .to_string();
    let dest = repo_dir.join(&filename);
    tokio::fs::create_dir_all(&repo_dir).await?;
    tokio::fs::copy(&package, &dest).await?;

    // Ensure .minisig exists; if not, create it by signing the package
    let sig_path = repo_dir.join(format!("{filename}.minisig"));
    if !sig_path.exists() {
        let data = tokio::fs::read(&dest).await?;
        let sig = sps2_signing::minisign_sign_bytes(
            &data,
            &key,
            pass.as_deref(),
            Some("sps2 package signature"),
            Some(&filename),
        )?;
        tokio::fs::write(&sig_path, sig).await?;
    }

    // Rebuild and sign index
    update_indices(repo_dir, base_url, key, pass).await
}

async fn update_indices(
    repo_dir: PathBuf,
    base_url: String,
    key: PathBuf,
    pass: Option<String>,
) -> Result<(), Error> {
    let store = LocalStore::new(&repo_dir);
    let publisher = Publisher::new(store, base_url);
    let artifacts = publisher.scan_packages_local_dir(&repo_dir).await?;
    let index = publisher.build_index(&artifacts);
    publisher
        .publish_index(&index, &key, pass.as_deref())
        .await?;
    println!(
        "Updated index with {} packages in {}",
        artifacts.len(),
        repo_dir.display()
    );
    Ok(())
}

async fn repo_init(
    repo_dir: PathBuf,
    pubkey: Option<PathBuf>,
    generate: bool,
    out_secret: Option<PathBuf>,
    out_public: Option<PathBuf>,
    comment: Option<String>,
) -> Result<(), Error> {
    tokio::fs::create_dir_all(&repo_dir).await?;

    let pk_base64 = if let Some(pub_path) = pubkey {
        let content = tokio::fs::read_to_string(&pub_path).await?;
        repo_keys::extract_base64(&content)
    } else if generate {
        // Generate unencrypted keypair for local testing
        use minisign::KeyPair;
        let KeyPair { pk, sk } = KeyPair::generate_unencrypted_keypair()
            .map_err(|e| Error::internal(format!("keypair generation failed: {e}")))?;
        // Write secret key
        let sk_path = out_secret.expect("out_secret required with --generate");
        let sk_box = sk
            .to_box(None)
            .map_err(|e| Error::internal(format!("secret key serialize failed: {e}")))?;
        tokio::fs::write(&sk_path, sk_box.to_string()).await?;
        // Write public key
        let pk_path = out_public.expect("out_public required with --generate");
        let pk_box = pk
            .to_box()
            .map_err(|e| Error::internal(format!("public key serialize failed: {e}")))?;
        tokio::fs::write(&pk_path, pk_box.to_string()).await?;
        // Extract base64 from box
        repo_keys::extract_base64(&pk_box.to_string())
    } else {
        return Err(Error::internal(
            "Provide --pubkey <file> or --generate with --out-secret/--out-public",
        ));
    };

    let repo = repo_keys::make_single_key(pk_base64, comment)?;
    repo_keys::write_keys_json(&repo_dir, &repo).await?;
    println!("Initialized repo at {} with keys.json", repo_dir.display());
    Ok(())
}

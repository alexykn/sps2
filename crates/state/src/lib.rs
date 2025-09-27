#![warn(mismatched_lifetime_syntaxes)]
#![deny(clippy::pedantic, unsafe_code)]
#![allow(
    clippy::needless_raw_string_hashes,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::map_unwrap_or,
    clippy::unused_async,
    clippy::missing_panics_doc
)]
#![allow(clippy::module_name_repetitions)]

//! State management for sps2
//!
//! This crate manages the `SQLite` database that tracks system state,
//! installed packages, and enables atomic updates with rollback.

pub mod db;
pub mod file_models;
pub mod file_queries_runtime;
pub mod live_slots;
pub mod manager;
pub mod models;

#[cfg(feature = "runtime-queries")]
pub use manager::{StateManager, TransactionData};
pub mod queries {
    pub use crate::file_queries_runtime::*;
    pub use crate::queries_runtime::*;
}

#[cfg(feature = "runtime-queries")]
mod queries_runtime;

pub use file_models::{
    DeduplicationResult, FileMTimeTracker, FileMetadata, FileObject, FileReference,
    FileStorageStats, InstalledFile, PackageFileEntry,
};
pub use models::{Package, PackageRef, State, StoreRef};

use sps2_errors::Error;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::path::Path;
use std::time::Duration;

/// Create a new `SQLite` connection pool
///
/// # Errors
///
/// Returns an error if the database connection fails or configuration is invalid.
pub async fn create_pool(db_path: &Path) -> Result<Pool<Sqlite>, Error> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(30));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(|e| {
            Error::from(sps2_errors::StateError::DatabaseError {
                message: e.to_string(),
            })
        })?;

    if let Ok(mut conn) = pool.acquire().await {
        let _ = sqlx::query("PRAGMA synchronous = NORMAL")
            .execute(&mut *conn)
            .await;
        let _ = sqlx::query("PRAGMA temp_store = MEMORY")
            .execute(&mut *conn)
            .await;
        let _ = sqlx::query("PRAGMA mmap_size = 268435456")
            .execute(&mut *conn)
            .await;
        let _ = sqlx::query("PRAGMA cache_size = -20000")
            .execute(&mut *conn)
            .await;
        let _ = sqlx::query("PRAGMA wal_autocheckpoint = 1000")
            .execute(&mut *conn)
            .await;
    }

    Ok(pool)
}

/// Run database migrations
///
/// # Errors
///
/// Returns an error if any migration fails to execute.
pub async fn run_migrations(pool: &Pool<Sqlite>) -> Result<(), Error> {
    sqlx::migrate!("./migrations").run(pool).await.map_err(|e| {
        sps2_errors::StateError::MigrationFailed {
            message: e.to_string(),
        }
        .into()
    })
}

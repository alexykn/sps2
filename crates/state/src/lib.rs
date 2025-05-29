#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! State management for spsv2
//!
//! This crate manages the SQLite database that tracks system state,
//! installed packages, and enables atomic updates with rollback.

pub mod manager;
pub mod models;

#[cfg(feature = "runtime-queries")]
pub mod queries {
    pub use crate::queries_runtime::*;
}
#[cfg(not(feature = "runtime-queries"))]
pub mod queries;

#[cfg(feature = "runtime-queries")]
mod queries_runtime;

pub use manager::StateManager;
pub use models::{Package, PackageRef, State, StoreRef};

use spsv2_errors::Error;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::path::Path;
use std::time::Duration;

/// Create a new SQLite connection pool
pub async fn create_pool(db_path: &Path) -> Result<Pool<Sqlite>, Error> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(30));

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(|e| {
            spsv2_errors::StateError::DatabaseError {
                message: e.to_string(),
            }
            .into()
        })
}

/// Run database migrations
pub async fn run_migrations(pool: &Pool<Sqlite>) -> Result<(), Error> {
    sqlx::migrate!("./migrations").run(pool).await.map_err(|e| {
        spsv2_errors::StateError::MigrationFailed {
            message: e.to_string(),
        }
        .into()
    })
}

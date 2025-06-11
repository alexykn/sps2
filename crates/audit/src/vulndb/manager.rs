//! High-level vulnerability database manager for coordination

use sps2_errors::{AuditError, Error};
use sps2_events::EventSender;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

use super::database::VulnerabilityDatabase;
use super::schema::{configure_pragmas, create_tables, initialize_metadata};
use super::statistics::is_database_fresh;
use super::updater::update_database_from_sources;

/// Vulnerability database manager
pub struct VulnDbManager {
    /// Database path
    db_path: PathBuf,
    /// Connection pool
    pool: Option<SqlitePool>,
}

impl VulnDbManager {
    /// Create new vulnerability database manager
    pub fn new(db_path: impl AsRef<Path>) -> Result<Self, Error> {
        let db_path = db_path.as_ref().to_path_buf();

        Ok(Self {
            db_path,
            pool: None,
        })
    }

    /// Get default vulnerability database path
    pub fn default_path() -> PathBuf {
        PathBuf::from("/opt/pm/vulndb/vulndb.sqlite")
    }

    /// Initialize database connection
    pub async fn initialize(&mut self) -> Result<(), Error> {
        // Ensure database directory exists
        if let Some(parent) = self.db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let database_url = format!("sqlite:{}?mode=rwc", self.db_path.display());

        // Create connection pool with options
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .map_err(|e| AuditError::DatabaseError {
                message: format!("Failed to connect to database: {e}"),
            })?;

        // Configure database pragmas
        configure_pragmas(&pool).await?;

        // Create tables and indexes
        create_tables(&pool).await?;

        // Initialize metadata if this is a new database
        initialize_metadata(&pool).await?;

        self.pool = Some(pool);
        Ok(())
    }

    /// Get the vulnerability database
    pub async fn get_database(&self) -> Result<VulnerabilityDatabase, Error> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| AuditError::DatabaseError {
                message: "Database not initialized".to_string(),
            })?;

        Ok(VulnerabilityDatabase::new(pool.clone()))
    }

    /// Update vulnerability database from sources with event reporting
    ///
    /// # Panics
    ///
    /// Panics if the pool is `None` after initialization, which should never happen
    /// as `initialize()` sets up the pool or returns an error.
    pub async fn update(&mut self) -> Result<(), Error> {
        self.update_with_events(None).await
    }

    /// Update vulnerability database from sources with optional event reporting
    ///
    /// # Panics
    ///
    /// Panics if the pool is `None` after initialization, which should never happen
    /// as `initialize()` sets up the pool or returns an error.
    pub async fn update_with_events(
        &mut self,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        if self.pool.is_none() {
            self.initialize().await?;
        }

        let pool = self.pool.as_ref().expect("pool should be initialized");
        update_database_from_sources(pool, event_sender).await
    }

    /// Check if database is fresh (updated recently)
    pub async fn is_fresh(&self) -> Result<bool, Error> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| AuditError::DatabaseError {
                message: "Database not initialized".to_string(),
            })?;

        is_database_fresh(pool).await
    }
}

//! SQL queries for state operations

use crate::models::{Package, State, StoreRef};
use sps2_errors::{Error, StateError};
use sps2_types::StateId;
use sqlx::{query, query_as, Row, Sqlite, Transaction};

/// Get the current active state
pub async fn get_active_state(tx: &mut Transaction<'_, Sqlite>) -> Result<StateId, Error> {
    let row = query!("SELECT state_id FROM active_state WHERE id = 1")
        .fetch_optional(&mut **tx)
        .await?;

    match row {
        Some(r) => {
            let id = uuid::Uuid::parse_str(&r.state_id)
                .map_err(|e| Error::internal(format!("invalid state ID: {e}")))?;
            Ok(id)
        }
        None => Err(StateError::ActiveStateMissing.into()),
    }
}

/// Set the active state
pub async fn set_active_state(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<(), Error> {
    let id_str = state_id.to_string();
    let now = chrono::Utc::now().timestamp();

    query!(
        "INSERT OR REPLACE INTO active_state (id, state_id, updated_at) VALUES (1, ?, ?)",
        id_str,
        now
    )
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Create a new state
pub async fn create_state(
    tx: &mut Transaction<'_, Sqlite>,
    id: &StateId,
    parent: Option<&StateId>,
    operation: &str,
) -> Result<(), Error> {
    let id_str = id.to_string();
    let parent_str = parent.map(ToString::to_string);
    let now = chrono::Utc::now().timestamp();

    query!(
        "INSERT INTO states (id, parent_id, created_at, operation, success) VALUES (?, ?, ?, ?, 1)",
        id_str,
        parent_str,
        now,
        operation
    )
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Get packages in a state
pub async fn get_state_packages(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<Package>, Error> {
    let id_str = state_id.to_string();

    let packages = query_as!(
        Package,
        "SELECT id, state_id, name, version, hash, size, installed_at
         FROM packages WHERE state_id = ?",
        id_str
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(packages)
}

/// Add a package to a state
pub async fn add_package(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    name: &str,
    version: &str,
    hash: &str,
    size: i64,
) -> Result<i64, Error> {
    let id_str = state_id.to_string();
    let now = chrono::Utc::now().timestamp();

    let result = query!(
        "INSERT INTO packages (state_id, name, version, hash, size, installed_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        id_str,
        name,
        version,
        hash,
        size,
        now
    )
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Get or create store reference
pub async fn get_or_create_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    size: i64,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();

    query!(
        "INSERT INTO store_refs (hash, ref_count, size, created_at) VALUES (?, 0, ?, ?)
         ON CONFLICT(hash) DO NOTHING",
        hash,
        size,
        now
    )
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Increment store reference count
pub async fn increment_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<(), Error> {
    query!(
        "UPDATE store_refs SET ref_count = ref_count + 1 WHERE hash = ?",
        hash
    )
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Decrement store reference count
pub async fn decrement_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<i64, Error> {
    query!(
        "UPDATE store_refs SET ref_count = ref_count - 1 WHERE hash = ?",
        hash
    )
    .execute(&mut **tx)
    .await?;

    let row = query!("SELECT ref_count FROM store_refs WHERE hash = ?", hash)
        .fetch_one(&mut **tx)
        .await?;

    Ok(row.ref_count)
}

/// Get unreferenced store items
pub async fn get_unreferenced_store_items(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<StoreRef>, Error> {
    let items = query_as!(
        StoreRef,
        "SELECT hash, ref_count, size, created_at FROM store_refs WHERE ref_count <= 0"
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(items)
}

/// Delete unreferenced store items
pub async fn delete_unreferenced_store_items(
    tx: &mut Transaction<'_, Sqlite>,
    hash_strings: &[String],
) -> Result<(), Error> {
    for hash in hash_strings {
        query!("DELETE FROM store_refs WHERE hash = ?", hash)
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}

/// Get all states for history
pub async fn get_all_states(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<State>, Error> {
    let states = query_as!(
        State,
        r#"SELECT id, parent_id, created_at, operation,
           success as "success: bool", rollback_of
           FROM states ORDER BY created_at DESC"#
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(states)
}

/// Get states to clean up (based on retention policy)
pub async fn get_states_to_cleanup(
    tx: &mut Transaction<'_, Sqlite>,
    retention_count: i64,
    retention_days: i64,
) -> Result<Vec<String>, Error> {
    let cutoff_time = chrono::Utc::now().timestamp() - (retention_days * 86400);

    // Keep the N most recent states AND states newer than cutoff
    let states = query!(
        r#"
        SELECT id FROM states
        WHERE id NOT IN (
            SELECT id FROM states ORDER BY created_at DESC LIMIT ?
        )
        AND created_at < ?
        AND id NOT IN (
            SELECT state_id FROM active_state WHERE id = 1
        )
        "#,
        retention_count,
        cutoff_time
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(states.into_iter().map(|r| r.id).collect())
}

/// Get packages that depend on the given package
pub async fn get_package_dependents(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
) -> Result<Vec<String>, Error> {
    let dependents = query!(
        r#"
        SELECT DISTINCT p.name
        FROM packages p
        JOIN dependencies d ON p.id = d.package_id
        WHERE d.dep_name = ?
        "#,
        package_name
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(dependents.into_iter().map(|r| r.name).collect())
}

/// List all states (IDs only)
pub async fn list_states(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<StateId>, Error> {
    let rows = query!("SELECT id FROM states ORDER BY created_at DESC")
        .fetch_all(&mut **tx)
        .await?;

    let mut states = Vec::new();
    for row in rows {
        let id = uuid::Uuid::parse_str(&row.id)
            .map_err(|e| Error::internal(format!("invalid state ID: {e}")))?;
        states.push(id);
    }

    Ok(states)
}

/// List all states with details
pub async fn list_states_detailed(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<State>, Error> {
    get_all_states(tx).await
}

/// Get package names in a state
pub async fn get_state_package_names(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<String>, Error> {
    let id_str = state_id.to_string();

    let rows = query!("SELECT name FROM packages WHERE state_id = ?", id_str)
        .fetch_all(&mut **tx)
        .await?;

    Ok(rows.into_iter().map(|r| r.name).collect())
}

/// Get parent state ID for a given state
pub async fn get_parent_state_id(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Option<StateId>, Error> {
    let id_str = state_id.to_string();
    let row = query!("SELECT parent_id FROM states WHERE id = ?", id_str)
        .fetch_optional(&mut **tx)
        .await?;

    match row {
        Some(r) => match r.parent_id {
            Some(parent_str) => {
                let parent_uuid = uuid::Uuid::parse_str(&parent_str)
                    .map_err(|e| Error::internal(format!("invalid parent state ID: {e}")))?;
                Ok(Some(parent_uuid))
            }
            None => Ok(None),
        },
        None => Ok(None),
    }
}

/// Add a file to the package_files table
pub async fn add_package_file(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
    file_path: &str,
    is_directory: bool,
) -> Result<(), Error> {
    let id_str = state_id.to_string();

    query!(
        "INSERT INTO package_files (state_id, package_name, package_version, file_path, is_directory)
         VALUES (?, ?, ?, ?, ?)",
        id_str,
        package_name,
        package_version,
        file_path,
        is_directory
    )
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Get all files for a package in a specific state
pub async fn get_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<String>, Error> {
    let id_str = state_id.to_string();

    let files = query!(
        "SELECT file_path FROM package_files 
         WHERE state_id = ? AND package_name = ? AND package_version = ?
         ORDER BY file_path",
        id_str,
        package_name,
        package_version
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(files.into_iter().map(|r| r.file_path).collect())
}

/// Get all files for a package in the active state
pub async fn get_active_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<String>, Error> {
    let active_state = get_active_state(tx).await?;
    get_package_files(tx, &active_state, package_name, package_version).await
}

/// Remove all files for a package from package_files table
pub async fn remove_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<(), Error> {
    let id_str = state_id.to_string();

    query!(
        "DELETE FROM package_files 
         WHERE state_id = ? AND package_name = ? AND package_version = ?",
        id_str,
        package_name,
        package_version
    )
    .execute(&mut **tx)
    .await?;

    Ok(())
}

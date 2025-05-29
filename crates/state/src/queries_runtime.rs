//! Runtime SQL queries for state operations (temporary until sqlx prepare is run)

use crate::models::{Package, State, StoreRef};
use spsv2_errors::{Error, StateError};
use spsv2_types::StateId;
use sqlx::{query, Row, Sqlite, Transaction};

/// Get the current active state
pub async fn get_active_state(tx: &mut Transaction<'_, Sqlite>) -> Result<StateId, Error> {
    let row = query("SELECT state_id FROM active_state WHERE id = 1")
        .fetch_optional(&mut **tx)
        .await?;

    match row {
        Some(r) => {
            let state_id: String = r.get("state_id");
            let id = uuid::Uuid::parse_str(&state_id)
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

    query("INSERT OR REPLACE INTO active_state (id, state_id, updated_at) VALUES (1, ?1, ?2)")
        .bind(id_str)
        .bind(now)
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
    let parent_str = parent.map(|p| p.to_string());
    let now = chrono::Utc::now().timestamp();

    query(
        "INSERT INTO states (id, parent_id, created_at, operation, success) 
         VALUES (?1, ?2, ?3, ?4, 1)"
    )
    .bind(id_str)
    .bind(parent_str)
    .bind(now)
    .bind(operation)
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

    let rows = query(
        "SELECT id, state_id, name, version, hash, size, installed_at 
         FROM packages WHERE state_id = ?1"
    )
    .bind(id_str)
    .fetch_all(&mut **tx)
    .await?;

    let packages = rows
        .into_iter()
        .map(|row| Package {
            id: row.get("id"),
            state_id: row.get("state_id"),
            name: row.get("name"),
            version: row.get("version"),
            hash: row.get("hash"),
            size: row.get("size"),
            installed_at: row.get("installed_at"),
        })
        .collect();

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

    let result = query(
        "INSERT INTO packages (state_id, name, version, hash, size, installed_at) 
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
    )
    .bind(id_str)
    .bind(name)
    .bind(version)
    .bind(hash)
    .bind(size)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Remove a package from a state
pub async fn remove_package(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    name: &str,
) -> Result<(), Error> {
    let id_str = state_id.to_string();

    query("DELETE FROM packages WHERE state_id = ?1 AND name = ?2")
        .bind(id_str)
        .bind(name)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Get or create a store reference
pub async fn get_or_create_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    size: i64,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();

    query(
        "INSERT OR IGNORE INTO store_refs (hash, ref_count, size, created_at) 
         VALUES (?1, 0, ?2, ?3)"
    )
    .bind(hash)
    .bind(size)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Increment store reference count
pub async fn increment_store_ref(tx: &mut Transaction<'_, Sqlite>, hash: &str) -> Result<(), Error> {
    query("UPDATE store_refs SET ref_count = ref_count + 1 WHERE hash = ?1")
        .bind(hash)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Decrement store reference count
pub async fn decrement_store_ref(tx: &mut Transaction<'_, Sqlite>, hash: &str) -> Result<(), Error> {
    query("UPDATE store_refs SET ref_count = ref_count - 1 WHERE hash = ?1")
        .bind(hash)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Get unreferenced store items
pub async fn get_unreferenced_items(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<StoreRef>, Error> {
    let rows = query("SELECT hash, ref_count, size, created_at FROM store_refs WHERE ref_count <= 0")
        .fetch_all(&mut **tx)
        .await?;

    let items = rows
        .into_iter()
        .map(|row| StoreRef {
            hash: row.get("hash"),
            ref_count: row.get("ref_count"),
            size: row.get("size"),
            created_at: row.get("created_at"),
        })
        .collect();

    Ok(items)
}

/// Check if state exists
pub async fn state_exists(tx: &mut Transaction<'_, Sqlite>, state_id: &StateId) -> Result<bool, Error> {
    let id_str = state_id.to_string();
    let row = query("SELECT 1 FROM states WHERE id = ?1")
        .bind(id_str)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(row.is_some())
}

/// List all states
pub async fn list_states(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<StateId>, Error> {
    let rows = query("SELECT id FROM states ORDER BY created_at DESC")
        .fetch_all(&mut **tx)
        .await?;

    let mut states = Vec::new();
    for row in rows {
        let id_str: String = row.get("id");
        let id = uuid::Uuid::parse_str(&id_str)
            .map_err(|e| Error::internal(format!("invalid state ID: {e}")))?;
        states.push(id);
    }
    Ok(states)
}

/// Get package names in a state
pub async fn get_state_package_names(tx: &mut Transaction<'_, Sqlite>, state_id: &StateId) -> Result<Vec<String>, Error> {
    let id_str = state_id.to_string();
    let rows = query("SELECT name FROM packages WHERE state_id = ?1")
        .bind(id_str)
        .fetch_all(&mut **tx)
        .await?;

    let packages = rows.into_iter().map(|row| row.get("name")).collect();
    Ok(packages)
}

/// Get all states
pub async fn get_all_states(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<State>, Error> {
    let rows = query(
        r#"SELECT id, parent_id, created_at, operation, 
           success, rollback_of 
           FROM states ORDER BY created_at DESC"#
    )
    .fetch_all(&mut **tx)
    .await?;

    let states = rows
        .into_iter()
        .map(|row| State {
            id: row.get("id"),
            parent_id: row.get("parent_id"),
            created_at: row.get("created_at"),
            operation: row.get("operation"),
            success: row.get("success"),
            rollback_of: row.get("rollback_of"),
        })
        .collect();

    Ok(states)
}

/// Get states for cleanup
pub async fn get_states_for_cleanup(
    tx: &mut Transaction<'_, Sqlite>,
    keep_count: usize,
    cutoff_time: i64,
) -> Result<Vec<String>, Error> {
    let rows = query(
        r#"
        SELECT id FROM states 
        WHERE id NOT IN (
            SELECT state_id FROM active_state
            UNION
            SELECT id FROM states ORDER BY created_at DESC LIMIT ?1
        )
        AND created_at < ?2
        AND success = 1
        ORDER BY created_at ASC
        "#
    )
    .bind(keep_count as i64)
    .bind(cutoff_time)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows.into_iter().map(|r| r.get("id")).collect())
}

/// Delete a state
pub async fn delete_state(tx: &mut Transaction<'_, Sqlite>, state_id: &str) -> Result<(), Error> {
    query("DELETE FROM states WHERE id = ?1")
        .bind(state_id)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Get states to cleanup (alias for get_states_for_cleanup)
pub async fn get_states_to_cleanup(
    tx: &mut Transaction<'_, Sqlite>,
    keep_count: usize,
    cutoff_time: i64,
) -> Result<Vec<String>, Error> {
    get_states_for_cleanup(tx, keep_count, cutoff_time).await
}

/// Get unreferenced store items (alias)
pub async fn get_unreferenced_store_items(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<StoreRef>, Error> {
    get_unreferenced_items(tx).await
}

/// Delete unreferenced store items
pub async fn delete_unreferenced_store_items(
    tx: &mut Transaction<'_, Sqlite>,
    hashes: &[String],
) -> Result<(), Error> {
    for hash in hashes {
        query("DELETE FROM store_refs WHERE hash = ?1")
            .bind(hash)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Get packages that depend on the given package
pub async fn get_package_dependents(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
) -> Result<Vec<String>, Error> {
    let rows = query(
        r#"
        SELECT DISTINCT p.name
        FROM packages p
        JOIN dependencies d ON p.id = d.package_id
        WHERE d.dep_name = ?1
        "#
    )
    .bind(package_name)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows.into_iter().map(|r| r.get("name")).collect())
}

/// List all states with details
pub async fn list_states_detailed(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<State>, Error> {
    get_all_states(tx).await
}
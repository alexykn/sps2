//! Runtime SQL queries for state operations (temporary until sqlx prepare is run)

use crate::models::{Package, State, StoreRef};
use sps2_errors::{Error, StateError};
use sps2_types::StateId;
use sqlx::{query, Row, Sqlite, Transaction};

/// Get the current active state
///
/// # Errors
///
/// Returns an error if the database query fails or no active state is found.
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
///
/// # Errors
///
/// Returns an error if the database update fails.
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
///
/// # Errors
///
/// Returns an error if the database insert fails.
pub async fn create_state(
    tx: &mut Transaction<'_, Sqlite>,
    id: &StateId,
    parent: Option<&StateId>,
    operation: &str,
) -> Result<(), Error> {
    let id_str = id.to_string();
    let parent_str = parent.map(ToString::to_string);
    let now = chrono::Utc::now().timestamp();

    query(
        "INSERT INTO states (id, parent_id, created_at, operation, success)
         VALUES (?1, ?2, ?3, ?4, 1)",
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
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_state_packages(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<Package>, Error> {
    let id_str = state_id.to_string();

    let rows = query(
        "SELECT id, state_id, name, version, hash, size, installed_at, venv_path
         FROM packages WHERE state_id = ?1",
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
            venv_path: row.get("venv_path"),
        })
        .collect();

    Ok(packages)
}

/// Get all packages including from parent states
///
/// This follows the parent chain and returns all packages that are effectively
/// installed in the current state, including those inherited from parent states.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_all_active_packages(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<Package>, Error> {
    let id_str = state_id.to_string();

    // Use a recursive CTE to get all states in the parent chain
    let rows = query(
        r#"
        WITH RECURSIVE state_chain AS (
            -- Start with the current state
            SELECT id, parent_id FROM states WHERE id = ?1
            
            UNION ALL
            
            -- Recursively get parent states
            SELECT s.id, s.parent_id 
            FROM states s
            INNER JOIN state_chain sc ON s.id = sc.parent_id
        )
        SELECT DISTINCT p.id, p.state_id, p.name, p.version, p.hash, p.size, p.installed_at, p.venv_path
        FROM packages p
        INNER JOIN state_chain sc ON p.state_id = sc.id
        -- Group by name to handle package updates/removals
        -- We want the most recent version of each package
        WHERE p.id IN (
            SELECT MAX(p2.id)
            FROM packages p2
            INNER JOIN state_chain sc2 ON p2.state_id = sc2.id
            WHERE p2.name = p.name
            GROUP BY p2.name
        )
        ORDER BY p.name
        "#,
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
            venv_path: row.get("venv_path"),
        })
        .collect();

    Ok(packages)
}

/// Add a package to a state
///
/// # Errors
///
/// Returns an error if the database insert fails.
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
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
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
///
/// # Errors
///
/// Returns an error if the database delete fails.
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
///
/// # Errors
///
/// Returns an error if the database insert fails.
pub async fn get_or_create_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    size: i64,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();

    query(
        "INSERT OR IGNORE INTO store_refs (hash, ref_count, size, created_at)
         VALUES (?1, 0, ?2, ?3)",
    )
    .bind(hash)
    .bind(size)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Increment store reference count
///
/// # Errors
///
/// Returns an error if the database update fails.
pub async fn increment_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<(), Error> {
    query("UPDATE store_refs SET ref_count = ref_count + 1 WHERE hash = ?1")
        .bind(hash)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Decrement store reference count
///
/// # Errors
///
/// Returns an error if the database update fails.
pub async fn decrement_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<(), Error> {
    query("UPDATE store_refs SET ref_count = ref_count - 1 WHERE hash = ?1")
        .bind(hash)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Get unreferenced store items
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_unreferenced_items(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<StoreRef>, Error> {
    let rows =
        query("SELECT hash, ref_count, size, created_at FROM store_refs WHERE ref_count <= 0")
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
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn state_exists(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<bool, Error> {
    let id_str = state_id.to_string();
    let row = query("SELECT 1 FROM states WHERE id = ?1")
        .bind(id_str)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(row.is_some())
}

/// List all states
///
/// # Errors
///
/// Returns an error if the database query fails or state IDs are invalid.
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
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_state_package_names(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<String>, Error> {
    let id_str = state_id.to_string();
    let rows = query("SELECT name FROM packages WHERE state_id = ?1")
        .bind(id_str)
        .fetch_all(&mut **tx)
        .await?;

    let packages = rows.into_iter().map(|row| row.get("name")).collect();
    Ok(packages)
}

/// Get all states
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_all_states(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<State>, Error> {
    let rows = query(
        r"SELECT id, parent_id, created_at, operation,
           success, rollback_of
           FROM states ORDER BY created_at DESC",
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
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_states_for_cleanup(
    tx: &mut Transaction<'_, Sqlite>,
    keep_count: usize,
    cutoff_time: i64,
) -> Result<Vec<String>, Error> {
    let rows = query(
        r"
        SELECT id FROM states
        WHERE id NOT IN (
            SELECT id FROM states ORDER BY created_at DESC LIMIT ?1
        )
        AND created_at < ?2
        AND id NOT IN (
            SELECT state_id FROM active_state WHERE id = 1
        )
        AND success = 1
        ORDER BY created_at ASC
        ",
    )
    .bind(
        i64::try_from(keep_count)
            .map_err(|e| Error::internal(format!("keep_count too large: {e}")))?,
    )
    .bind(cutoff_time)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows.into_iter().map(|r| r.get("id")).collect())
}

/// Delete a state
///
/// # Errors
///
/// Returns an error if the database delete fails.
pub async fn delete_state(tx: &mut Transaction<'_, Sqlite>, state_id: &str) -> Result<(), Error> {
    query("DELETE FROM states WHERE id = ?1")
        .bind(state_id)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Get states to cleanup (alias for `get_states_for_cleanup`)
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_states_to_cleanup(
    tx: &mut Transaction<'_, Sqlite>,
    keep_count: usize,
    cutoff_time: i64,
) -> Result<Vec<String>, Error> {
    get_states_for_cleanup(tx, keep_count, cutoff_time).await
}

/// Get unreferenced store items (alias)
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_unreferenced_store_items(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<StoreRef>, Error> {
    get_unreferenced_items(tx).await
}

/// Delete unreferenced store items
///
/// # Errors
///
/// Returns an error if the database delete fails.
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
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_package_dependents(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
) -> Result<Vec<String>, Error> {
    let rows = query(
        r"
        SELECT DISTINCT p.name
        FROM packages p
        JOIN dependencies d ON p.id = d.package_id
        WHERE d.dep_name = ?1
        ",
    )
    .bind(package_name)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows.into_iter().map(|r| r.get("name")).collect())
}

/// List all states with details
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn list_states_detailed(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<State>, Error> {
    get_all_states(tx).await
}

/// Get parent state ID for a given state
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_parent_state_id(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Option<StateId>, Error> {
    let id_str = state_id.to_string();
    let row = query("SELECT parent_id FROM states WHERE id = ?1")
        .bind(id_str)
        .fetch_optional(&mut **tx)
        .await?;

    match row {
        Some(r) => {
            let parent_id: Option<String> = r.get("parent_id");
            match parent_id {
                Some(parent_str) => {
                    let parent_uuid = uuid::Uuid::parse_str(&parent_str)
                        .map_err(|e| Error::internal(format!("invalid parent state ID: {e}")))?;
                    Ok(Some(parent_uuid))
                }
                None => Ok(None),
            }
        }
        None => Ok(None),
    }
}

/// Add a file to the `package_files` table
///
/// # Errors
///
/// Returns an error if the database insert fails.
pub async fn add_package_file(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
    file_path: &str,
    is_directory: bool,
) -> Result<(), Error> {
    let id_str = state_id.to_string();

    query(
        "INSERT INTO package_files (state_id, package_name, package_version, file_path, is_directory)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(id_str)
    .bind(package_name)
    .bind(package_version)
    .bind(file_path)
    .bind(is_directory)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Get all files for a package in a specific state
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<String>, Error> {
    let id_str = state_id.to_string();

    let rows = query(
        "SELECT file_path FROM package_files 
         WHERE state_id = ?1 AND package_name = ?2 AND package_version = ?3
         ORDER BY file_path",
    )
    .bind(id_str)
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows.into_iter().map(|r| r.get("file_path")).collect())
}

/// Get package files including from parent states
///
/// This follows the parent chain to find files for a package that might be
/// defined in a parent state rather than the current state.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_package_files_with_inheritance(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<String>, Error> {
    let id_str = state_id.to_string();

    // Use a recursive CTE to get all states in the parent chain
    let rows = query(
        r#"
        WITH RECURSIVE state_chain AS (
            -- Start with the current state
            SELECT id, parent_id FROM states WHERE id = ?1
            
            UNION ALL
            
            -- Recursively get parent states
            SELECT s.id, s.parent_id 
            FROM states s
            INNER JOIN state_chain sc ON s.id = sc.parent_id
        )
        SELECT DISTINCT pf.file_path
        FROM package_files pf
        INNER JOIN state_chain sc ON pf.state_id = sc.id
        WHERE pf.package_name = ?2 AND pf.package_version = ?3
        ORDER BY pf.file_path
        "#,
    )
    .bind(id_str)
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows.into_iter().map(|r| r.get("file_path")).collect())
}

/// Get all files for a package in the active state
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_active_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<String>, Error> {
    let active_state = get_active_state(tx).await?;
    get_package_files(tx, &active_state, package_name, package_version).await
}

/// Remove all files for a package from `package_files` table
///
/// # Errors
///
/// Returns an error if the database delete fails.
pub async fn remove_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<(), Error> {
    let id_str = state_id.to_string();

    query(
        "DELETE FROM package_files 
         WHERE state_id = ?1 AND package_name = ?2 AND package_version = ?3",
    )
    .bind(id_str)
    .bind(package_name)
    .bind(package_version)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Insert a garbage collection log entry
///
/// # Errors
///
/// Returns an error if the database insert fails.
pub async fn insert_gc_log(
    tx: &mut Transaction<'_, Sqlite>,
    items_removed: i64,
    space_freed: i64,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();

    query("INSERT INTO gc_log (run_at, items_removed, space_freed) VALUES (?1, ?2, ?3)")
        .bind(now)
        .bind(items_removed)
        .bind(space_freed)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Add a package to a state with venv path
///
/// # Errors
///
/// Returns an error if the database insert fails.
pub async fn add_package_with_venv(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    name: &str,
    version: &str,
    hash: &str,
    size: i64,
    venv_path: Option<&str>,
) -> Result<i64, Error> {
    let id_str = state_id.to_string();
    let now = chrono::Utc::now().timestamp();

    let result = query(
        "INSERT INTO packages (state_id, name, version, hash, size, installed_at, venv_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(id_str)
    .bind(name)
    .bind(version)
    .bind(hash)
    .bind(size)
    .bind(now)
    .bind(venv_path)
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Get the venv path for a package
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_package_venv_path(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<Option<String>, Error> {
    let id_str = state_id.to_string();

    let row = query(
        "SELECT venv_path FROM packages 
         WHERE state_id = ?1 AND name = ?2 AND version = ?3",
    )
    .bind(id_str)
    .bind(package_name)
    .bind(package_version)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(row.and_then(|r| r.get("venv_path")))
}

/// Get all packages with venvs in a state
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_packages_with_venvs(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<(String, String, String)>, Error> {
    let id_str = state_id.to_string();

    let rows = query(
        "SELECT name, version, venv_path 
         FROM packages 
         WHERE state_id = ?1 AND venv_path IS NOT NULL",
    )
    .bind(id_str)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let name: String = r.get("name");
            let version: String = r.get("version");
            let venv_path: Option<String> = r.get("venv_path");
            venv_path.map(|venv| (name, version, venv))
        })
        .collect())
}

/// Update venv path for a package
///
/// # Errors
///
/// Returns an error if the database update fails.
pub async fn update_package_venv_path(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
    venv_path: Option<&str>,
) -> Result<(), Error> {
    let id_str = state_id.to_string();

    query(
        "UPDATE packages SET venv_path = ?1
         WHERE state_id = ?2 AND name = ?3 AND version = ?4",
    )
    .bind(venv_path)
    .bind(id_str)
    .bind(package_name)
    .bind(package_version)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Add a package to the package map
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn add_package_map(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    version: &str,
    hash: &str,
) -> Result<(), Error> {
    let now = chrono::Utc::now().to_rfc3339();

    query(
        "INSERT OR REPLACE INTO package_map (name, version, hash, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(name)
    .bind(version)
    .bind(hash)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Get the hash for a package name and version
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_package_hash(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    version: &str,
) -> Result<Option<String>, Error> {
    let row = query("SELECT hash FROM package_map WHERE name = ?1 AND version = ?2")
        .bind(name)
        .bind(version)
        .fetch_optional(&mut **tx)
        .await?;

    Ok(row.map(|r| r.get("hash")))
}

/// Remove a package from the package map
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn remove_package_map(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    version: &str,
) -> Result<(), Error> {
    query("DELETE FROM package_map WHERE name = ?1 AND version = ?2")
        .bind(name)
        .bind(version)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

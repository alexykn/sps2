//! Runtime SQL queries for state operations (schema v2)

use crate::models::{Package, State, StoreRef};
use sps2_errors::{Error, StateError};
use sps2_types::StateId;
use sqlx::{query, Row, Sqlite, Transaction};
use std::collections::HashMap;
use std::convert::TryFrom;

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

/// Set the active state pointer
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

/// Insert a new state row
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
        "INSERT INTO states (id, parent_id, created_at, operation, success) VALUES (?1, ?2, ?3, ?4, 1)",
    )
    .bind(id_str)
    .bind(parent_str)
    .bind(now)
    .bind(operation)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Get packages present in a particular state snapshot
pub async fn get_state_packages(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<Package>, Error> {
    let id_str = state_id.to_string();
    let rows = query(
        r#"
        SELECT
            sp.id              AS pkg_row_id,
            sp.state_id        AS state_id,
            pv.name            AS name,
            pv.version         AS version,
            pv.store_hash      AS hash,
            pv.size_bytes      AS size,
            sp.added_at        AS installed_at
        FROM state_packages sp
        JOIN package_versions pv ON pv.id = sp.package_version_id
        WHERE sp.state_id = ?1
        ORDER BY pv.name
        "#,
    )
    .bind(id_str)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Package {
            id: row.get("pkg_row_id"),
            state_id: row.get("state_id"),
            name: row.get("name"),
            version: row.get("version"),
            hash: row.get("hash"),
            size: row.get("size"),
            installed_at: row.get("installed_at"),
            venv_path: None,
        })
        .collect())
}

/// All packages for a state (same as get_state_packages under v2 schema)
pub async fn get_all_active_packages(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<Package>, Error> {
    get_state_packages(tx, state_id).await
}

/// Ensure a package version exists and add it to a state snapshot
pub async fn add_package(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    name: &str,
    version: &str,
    store_hash: &str,
    size: i64,
) -> Result<i64, Error> {
    let id_str = state_id.to_string();
    let now = chrono::Utc::now().timestamp();

    query(
        r#"
        INSERT INTO package_versions (name, version, store_hash, size_bytes, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(name, version) DO UPDATE SET
            store_hash = excluded.store_hash,
            size_bytes = excluded.size_bytes
        "#,
    )
    .bind(name)
    .bind(version)
    .bind(store_hash)
    .bind(size)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    query(
        r#"
        INSERT INTO state_packages (state_id, package_version_id, install_size_bytes, added_at)
        VALUES (?1,
            (SELECT id FROM package_versions WHERE name = ?2 AND version = ?3),
            ?4,
            ?5)
        "#,
    )
    .bind(&id_str)
    .bind(name)
    .bind(version)
    .bind(size)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    let row = query("SELECT last_insert_rowid() as id")
        .fetch_one(&mut **tx)
        .await?;
    Ok(row.get("id"))
}

/// Remove a package version reference from a state snapshot
pub async fn remove_package(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    name: &str,
) -> Result<(), Error> {
    let id_str = state_id.to_string();
    query(
        r#"
        DELETE FROM state_packages
        WHERE state_id = ?1
          AND package_version_id IN (SELECT id FROM package_versions WHERE name = ?2)
        "#,
    )
    .bind(id_str)
    .bind(name)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Ensure an archive CAS row exists
pub async fn get_or_create_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    size: i64,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();
    query(
        r#"
        INSERT OR IGNORE INTO cas_objects (hash, kind, size_bytes, created_at, ref_count)
        VALUES (?1, 'archive', ?2, ?3, 0)
        "#,
    )
    .bind(hash)
    .bind(size)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Increment archive refcount
pub async fn increment_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<(), Error> {
    query("UPDATE cas_objects SET ref_count = ref_count + 1 WHERE hash = ?1 AND kind = 'archive'")
        .bind(hash)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Decrement archive refcount
pub async fn decrement_store_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<(), Error> {
    query("UPDATE cas_objects SET ref_count = ref_count - 1 WHERE hash = ?1 AND kind = 'archive'")
        .bind(hash)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Force-set archive refcount
pub async fn set_store_ref_count(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    count: i64,
) -> Result<u64, Error> {
    let res = query(
        "UPDATE cas_objects SET ref_count = ?1 WHERE hash = ?2 AND kind = 'archive' AND ref_count <> ?1",
    )
    .bind(count)
    .bind(hash)
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected())
}

/// Fetch archive CAS rows with refcount <= 0
pub async fn get_unreferenced_items(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<StoreRef>, Error> {
    let rows = query(
        r#"
        SELECT hash, ref_count, size_bytes AS size, created_at
        FROM cas_objects
        WHERE kind = 'archive' AND ref_count <= 0
        "#,
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| StoreRef {
            hash: row.get("hash"),
            ref_count: row.get("ref_count"),
            size: row.get("size"),
            created_at: row.get("created_at"),
        })
        .collect())
}

/// Check whether a given state exists
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

/// List state IDs ordered by creation time (desc)
pub async fn list_states(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<StateId>, Error> {
    let rows = query("SELECT id FROM states ORDER BY created_at DESC")
        .fetch_all(&mut **tx)
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for r in rows {
        let id: String = r.get("id");
        result.push(
            uuid::Uuid::parse_str(&id)
                .map_err(|e| Error::internal(format!("invalid state ID: {e}")))?,
        );
    }
    Ok(result)
}

/// List state names for a given state (unique package names)
pub async fn get_state_package_names(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
) -> Result<Vec<String>, Error> {
    let id_str = state_id.to_string();
    let rows = query(
        r#"
        SELECT DISTINCT pv.name
        FROM state_packages sp
        JOIN package_versions pv ON pv.id = sp.package_version_id
        WHERE sp.state_id = ?1
        ORDER BY pv.name
        "#,
    )
    .bind(id_str)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|r| r.get("name")).collect())
}

/// List all states with metadata
pub async fn get_all_states(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<State>, Error> {
    let rows = query(
        r#"
        SELECT id, parent_id, created_at, operation, success, rollback_of, pruned_at
        FROM states
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| State {
            id: row.get("id"),
            parent_id: row.get("parent_id"),
            created_at: row.get("created_at"),
            operation: row.get("operation"),
            success: row.get("success"),
            rollback_of: row.get("rollback_of"),
            pruned_at: row.get("pruned_at"),
        })
        .collect())
}

/// States eligible for cleanup by age and retention count
pub async fn get_states_for_cleanup(
    tx: &mut Transaction<'_, Sqlite>,
    keep_count: usize,
    cutoff_time: i64,
) -> Result<Vec<String>, Error> {
    let rows = query(
        r#"
        SELECT id FROM states
        WHERE id NOT IN (
            SELECT id FROM states ORDER BY created_at DESC LIMIT ?1
        )
        AND created_at < ?2
        AND id NOT IN (SELECT state_id FROM active_state WHERE id = 1)
        AND success = 1
        ORDER BY created_at ASC
        "#,
    )
    .bind(i64::try_from(keep_count).map_err(|e| Error::internal(e.to_string()))?)
    .bind(cutoff_time)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|r| r.get("id")).collect())
}

/// Delete a state row
pub async fn delete_state(tx: &mut Transaction<'_, Sqlite>, state_id: &str) -> Result<(), Error> {
    query("DELETE FROM states WHERE id = ?1")
        .bind(state_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Alias for get_states_for_cleanup
pub async fn get_states_to_cleanup(
    tx: &mut Transaction<'_, Sqlite>,
    keep_count: usize,
    cutoff_time: i64,
) -> Result<Vec<String>, Error> {
    get_states_for_cleanup(tx, keep_count, cutoff_time).await
}

/// Strict retention: keep only N newest states
pub async fn get_states_for_cleanup_strict(
    tx: &mut Transaction<'_, Sqlite>,
    keep_count: usize,
) -> Result<Vec<String>, Error> {
    let rows = query(
        r#"
        SELECT id FROM states
        WHERE id NOT IN (
            SELECT id FROM states ORDER BY created_at DESC LIMIT ?1
        )
        AND id NOT IN (SELECT state_id FROM active_state WHERE id = 1)
        AND success = 1
        ORDER BY created_at ASC
        "#,
    )
    .bind(i64::try_from(keep_count).map_err(|e| Error::internal(e.to_string()))?)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|r| r.get("id")).collect())
}

/// Alias for get_unreferenced_items (kept for callers)
pub async fn get_unreferenced_store_items(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<StoreRef>, Error> {
    get_unreferenced_items(tx).await
}

/// Delete archive CAS rows for given hashes
pub async fn delete_unreferenced_store_items(
    tx: &mut Transaction<'_, Sqlite>,
    hashes: &[String],
) -> Result<(), Error> {
    for hash in hashes {
        query("DELETE FROM cas_objects WHERE hash = ?1 AND kind = 'archive'")
            .bind(hash)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Fetch all archive CAS rows
pub async fn get_all_store_refs(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<StoreRef>, Error> {
    let rows = query(
        r#"SELECT hash, ref_count, size_bytes AS size, created_at FROM cas_objects WHERE kind = 'archive'"#,
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| StoreRef {
            hash: row.get("hash"),
            ref_count: row.get("ref_count"),
            size: row.get("size"),
            created_at: row.get("created_at"),
        })
        .collect())
}

/// Map archive hash -> last reference timestamp
pub async fn get_package_last_ref_map(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<HashMap<String, i64>, Error> {
    let rows = query(
        r#"
        SELECT pv.store_hash AS hash, COALESCE(MAX(s.created_at), 0) AS last_ref
        FROM state_packages sp
        JOIN package_versions pv ON pv.id = sp.package_version_id
        JOIN states s ON s.id = sp.state_id
        GROUP BY pv.store_hash
        "#,
    )
    .fetch_all(&mut **tx)
    .await?;

    let mut map = HashMap::new();
    for row in rows {
        map.insert(row.get("hash"), row.get("last_ref"));
    }
    Ok(map)
}

/// Insert archive eviction log entry
pub async fn insert_package_eviction(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    size: i64,
    reason: Option<&str>,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();
    query(
        r#"
        INSERT OR REPLACE INTO cas_evictions (hash, kind, evicted_at, size_bytes, reason)
        VALUES (?1, 'archive', ?2, ?3, ?4)
        "#,
    )
    .bind(hash)
    .bind(now)
    .bind(size)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Insert file eviction log entry
pub async fn insert_file_object_eviction(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    size: i64,
    reason: Option<&str>,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();
    query(
        r#"
        INSERT OR REPLACE INTO cas_evictions (hash, kind, evicted_at, size_bytes, reason)
        VALUES (?1, 'file', ?2, ?3, ?4)
        "#,
    )
    .bind(hash)
    .bind(now)
    .bind(size)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// List package names that depend on the given package name across all versions
pub async fn get_package_dependents(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
) -> Result<Vec<String>, Error> {
    let rows = query(
        r#"
        SELECT DISTINCT pv.name
        FROM package_versions pv
        JOIN package_deps d ON d.package_version_id = pv.id
        WHERE d.dep_name = ?1
        ORDER BY pv.name
        "#,
    )
    .bind(package_name)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|r| r.get("name")).collect())
}

/// Detailed state list (alias for get_all_states)
pub async fn list_states_detailed(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<State>, Error> {
    get_all_states(tx).await
}

/// States older than cutoff timestamp
pub async fn get_states_older_than(
    tx: &mut Transaction<'_, Sqlite>,
    cutoff: i64,
) -> Result<Vec<String>, Error> {
    let rows = query("SELECT id FROM states WHERE created_at < ? ORDER BY created_at ASC")
        .bind(cutoff)
        .fetch_all(&mut **tx)
        .await?;
    Ok(rows.into_iter().map(|r| r.get("id")).collect())
}

/// Mark states as pruned (except the active one)
pub async fn mark_pruned_states(
    tx: &mut Transaction<'_, Sqlite>,
    ids: &[String],
    ts: i64,
    active_id: &str,
) -> Result<usize, Error> {
    let mut updated = 0usize;
    for id in ids {
        if id == active_id {
            continue;
        }
        let res = query("UPDATE states SET pruned_at = ?1 WHERE id = ?2 AND pruned_at IS NULL")
            .bind(ts)
            .bind(id)
            .execute(&mut **tx)
            .await?;
        if res.rows_affected() > 0 {
            updated += 1;
        }
    }
    Ok(updated)
}

/// Clear pruned marker for a state
pub async fn unprune_state(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<(), Error> {
    query("UPDATE states SET pruned_at = NULL WHERE id = ?1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Fetch parent state ID if any
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
            let parent: Option<String> = r.get("parent_id");
            if let Some(p) = parent {
                let id = uuid::Uuid::parse_str(&p)
                    .map_err(|e| Error::internal(format!("invalid parent state ID: {e}")))?;
                Ok(Some(id))
            } else {
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

/// Legacy helper: record package files for directory entries (no-op for new schema)
pub async fn add_package_file(
    tx: &mut Transaction<'_, Sqlite>,
    _state_id: &StateId,
    package_name: &str,
    package_version: &str,
    file_path: &str,
    is_directory: bool,
) -> Result<(), Error> {
    let mode = if is_directory { 0 } else { 0o644 };
    query(
        r#"
        INSERT OR IGNORE INTO package_files
          (package_version_id, file_hash, rel_path, mode, uid, gid, mtime)
        VALUES (
          (SELECT id FROM package_versions WHERE name = ?1 AND version = ?2),
          '', ?3, ?4, 0, 0, NULL
        )
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .bind(file_path)
    .bind(i64::from(mode))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Fetch package file paths for a given version
pub async fn get_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    _state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<String>, Error> {
    let rows = query(
        r#"
        SELECT pf.rel_path
        FROM package_files pf
        JOIN package_versions pv ON pv.id = pf.package_version_id
        WHERE pv.name = ?1 AND pv.version = ?2
        ORDER BY pf.rel_path
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|r| r.get("rel_path")).collect())
}

/// Fetch package files ensuring version is present in state
pub async fn get_package_files_with_inheritance(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<String>, Error> {
    let id_str = state_id.to_string();
    let exists = query(
        r#"
        SELECT 1
        FROM state_packages sp
        JOIN package_versions pv ON pv.id = sp.package_version_id
        WHERE sp.state_id = ?1 AND pv.name = ?2 AND pv.version = ?3
        LIMIT 1
        "#,
    )
    .bind(id_str)
    .bind(package_name)
    .bind(package_version)
    .fetch_optional(&mut **tx)
    .await?
    .is_some();

    if !exists {
        return Ok(Vec::new());
    }
    get_package_files(tx, state_id, package_name, package_version).await
}

/// Package files for the active state
pub async fn get_active_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<String>, Error> {
    let active = get_active_state(tx).await?;
    get_package_files(tx, &active, package_name, package_version).await
}

/// Remove package files for a given version
pub async fn remove_package_files(
    tx: &mut Transaction<'_, Sqlite>,
    _state_id: &StateId,
    package_name: &str,
    package_version: &str,
) -> Result<(), Error> {
    query(
        r#"
        DELETE FROM package_files
        WHERE package_version_id = (SELECT id FROM package_versions WHERE name = ?1 AND version = ?2)
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Insert GC run entry
pub async fn insert_gc_log(
    tx: &mut Transaction<'_, Sqlite>,
    items_removed: i64,
    space_freed: i64,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();
    query(
        r#"
        INSERT INTO gc_runs (run_at, scope, items_removed, bytes_freed, notes)
        VALUES (?1, 'both', ?2, ?3, NULL)
        "#,
    )
    .bind(now)
    .bind(items_removed)
    .bind(space_freed)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Add package with venv path (venv ignored in v2 schema)
pub async fn add_package_with_venv(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &StateId,
    name: &str,
    version: &str,
    store_hash: &str,
    size: i64,
    _venv_path: Option<&str>,
) -> Result<i64, Error> {
    add_package(tx, state_id, name, version, store_hash, size).await
}

/// Venv path lookup (always None now)
pub async fn get_package_venv_path(
    _tx: &mut Transaction<'_, Sqlite>,
    _state_id: &StateId,
    _package_name: &str,
    _package_version: &str,
) -> Result<Option<String>, Error> {
    Ok(None)
}

/// Packages with venvs (empty under v2 schema)
pub async fn get_packages_with_venvs(
    _tx: &mut Transaction<'_, Sqlite>,
    _state_id: &StateId,
) -> Result<Vec<(String, String, String)>, Error> {
    Ok(Vec::new())
}

/// Update venv path (no-op)
pub async fn update_package_venv_path(
    _tx: &mut Transaction<'_, Sqlite>,
    _state_id: &StateId,
    _package_name: &str,
    _package_version: &str,
    _venv_path: Option<&str>,
) -> Result<(), Error> {
    Ok(())
}

/// Record package mapping (now writes to package_versions)
pub async fn add_package_map(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    version: &str,
    store_hash: &str,
    package_hash: Option<&str>,
) -> Result<(), Error> {
    let now = chrono::Utc::now().timestamp();
    query(
        r#"
        INSERT INTO package_versions (name, version, store_hash, package_hash, size_bytes, created_at)
        VALUES (?1, ?2, ?3, ?4, 0, ?5)
        ON CONFLICT(name, version) DO UPDATE SET
            store_hash = excluded.store_hash,
            package_hash = excluded.package_hash
        "#,
    )
    .bind(name)
    .bind(version)
    .bind(store_hash)
    .bind(package_hash)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Lookup store hash by name+version
pub async fn get_package_hash(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    version: &str,
) -> Result<Option<String>, Error> {
    let row = query("SELECT store_hash FROM package_versions WHERE name = ?1 AND version = ?2")
        .bind(name)
        .bind(version)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(row.map(|r| r.get("store_hash")))
}

/// Lookup store hash by package archive hash
pub async fn get_store_hash_for_package_hash(
    tx: &mut Transaction<'_, Sqlite>,
    package_hash: &str,
) -> Result<Option<String>, Error> {
    let row = query("SELECT store_hash FROM package_versions WHERE package_hash = ?1")
        .bind(package_hash)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(row.map(|r| r.get("store_hash")))
}

/// Remove package mapping entry
pub async fn remove_package_map(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    version: &str,
) -> Result<(), Error> {
    query("DELETE FROM package_versions WHERE name = ?1 AND version = ?2")
        .bind(name)
        .bind(version)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

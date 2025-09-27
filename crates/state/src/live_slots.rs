//! Slot management for the live prefix.
//!
//! Slots provide two alternating directories that back the `/opt/pm/live`
//! filesystem tree. Every operation stages into the inactive slot, commits do a
//! pair of atomic renames so `/opt/pm/live` always remains a real directory, and
//! the inactive slot retains the previous state for rollback.

use serde::{Deserialize, Serialize};
use sps2_errors::Error;
use sps2_platform::filesystem_helpers as fs;
use sps2_types::state::SlotId;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tokio::fs as tokio_fs;
use uuid::Uuid;

const SLOT_STATE_FILENAME: &str = "STATE";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SlotMetadata {
    active: SlotId,
    #[serde(default)]
    live_a: Option<Uuid>,
    #[serde(default)]
    live_b: Option<Uuid>,
}

impl Default for SlotMetadata {
    fn default() -> Self {
        Self {
            active: SlotId::A,
            live_a: None,
            live_b: None,
        }
    }
}

impl SlotMetadata {
    fn state(&self, slot: SlotId) -> Option<Uuid> {
        match slot {
            SlotId::A => self.live_a,
            SlotId::B => self.live_b,
        }
    }

    fn set_state(&mut self, slot: SlotId, value: Option<Uuid>) {
        match slot {
            SlotId::A => self.live_a = value,
            SlotId::B => self.live_b = value,
        }
    }
}

/// Manages alternating live slots.
pub struct LiveSlots {
    live_path: PathBuf,
    slots_dir: PathBuf,
    metadata_path: PathBuf,
    metadata: SlotMetadata,
}

impl LiveSlots {
    /// Initialize slot tracking.
    pub async fn initialize(state_dir: PathBuf, live_path: PathBuf) -> Result<Self, Error> {
        let slots_dir = state_dir.join("slots");
        fs::create_dir_all(&slots_dir).await?;
        for slot in SlotId::ALL {
            fs::create_dir_all(&slots_dir.join(slot.dir_name())).await?;
        }

        if !fs::exists(&live_path).await {
            fs::create_dir_all(&live_path).await?;
        }

        let metadata_path = state_dir.join("live_slots.json");
        let metadata = Self::load_metadata(&metadata_path).await?;

        let slots = Self {
            live_path,
            slots_dir,
            metadata_path,
            metadata,
        };

        slots.persist_metadata().await?;
        Ok(slots)
    }

    /// Returns the currently active slot.
    pub fn active_slot(&self) -> SlotId {
        self.metadata.active
    }

    /// Returns the inactive slot (target for staging).
    pub fn inactive_slot(&self) -> SlotId {
        self.metadata.active.other()
    }

    /// Filesystem path for a slot directory.
    pub fn slot_path(&self, slot: SlotId) -> PathBuf {
        self.slots_dir.join(slot.dir_name())
    }

    /// Lookup the recorded state for a slot.
    pub fn slot_state(&self, slot: SlotId) -> Option<Uuid> {
        self.metadata.state(slot)
    }

    /// Ensure a slot directory exists and return its path.
    pub async fn ensure_slot_dir(&mut self, slot: SlotId) -> Result<PathBuf, Error> {
        let path = self.slot_path(slot);
        fs::create_dir_all(&path).await?;
        Ok(path)
    }

    /// Record (or clear) the state marker for a slot and persist metadata.
    pub async fn record_slot_state(
        &mut self,
        slot: SlotId,
        state: Option<Uuid>,
    ) -> Result<(), Error> {
        self.write_slot_marker(slot, state).await?;
        self.metadata.set_state(slot, state);
        self.persist_metadata().await
    }

    /// Update metadata without touching markers.
    pub async fn set_slot_state(&mut self, slot: SlotId, state: Option<Uuid>) -> Result<(), Error> {
        self.metadata.set_state(slot, state);
        self.persist_metadata().await
    }

    /// Refresh state markers from disk.
    pub async fn refresh_slot_states(&mut self) -> Result<(), Error> {
        for slot in SlotId::ALL {
            let marker_path = self.slot_path(slot).join(SLOT_STATE_FILENAME);
            let state = match tokio_fs::read_to_string(&marker_path).await {
                Ok(content) => parse_state_marker(&content)?,
                Err(err) if err.kind() == ErrorKind::NotFound => None,
                Err(err) => {
                    return Err(Error::internal(format!(
                        "failed to read slot marker {}: {err}",
                        marker_path.display()
                    )))
                }
            };
            self.metadata.set_state(slot, state);
        }
        self.persist_metadata().await
    }

    /// Swap the prepared slot into `/opt/pm/live`, preserving the previous live
    /// directory under the previously active slot path.
    pub async fn swap_to_live(
        &mut self,
        staging_slot: SlotId,
        new_state: Uuid,
        parent_state: Uuid,
    ) -> Result<(), Error> {
        let current_active = self.metadata.active;
        let live_path = self.live_path.clone();
        let staging_path = self.slot_path(staging_slot);
        let backup_path = self.slot_path(current_active);

        if fs::exists(&backup_path).await {
            fs::remove_dir_all(&backup_path).await?;
        }

        if fs::exists(&live_path).await {
            fs::atomic_rename(&live_path, &backup_path).await?;
        }

        if let Some(parent) = live_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::atomic_rename(&staging_path, &live_path).await?;

        // Recreate an empty directory for the slot we just promoted so future
        // operations can stage into it once it becomes inactive again.
        fs::create_dir_all(&staging_path).await?;
        self.write_slot_marker(staging_slot, Some(new_state)).await?;

        self.metadata.active = staging_slot;
        self.metadata.set_state(staging_slot, Some(new_state));
        self.metadata.set_state(current_active, Some(parent_state));
        self.persist_metadata().await
    }

    async fn write_slot_marker(&self, slot: SlotId, state: Option<Uuid>) -> Result<(), Error> {
        let marker_path = self.slot_path(slot).join(SLOT_STATE_FILENAME);
        match state {
            Some(id) => {
                tokio_fs::write(&marker_path, id.to_string())
                    .await
                    .map_err(|e| Error::internal(format!("failed to write slot marker: {e}")))?
            }
            None => {
                if fs::exists(&marker_path).await {
                    fs::remove_file(&marker_path).await?;
                }
            }
        }
        Ok(())
    }

    async fn persist_metadata(&self) -> Result<(), Error> {
        let payload = serde_json::to_vec_pretty(&self.metadata)
            .map_err(|e| Error::internal(format!("failed to serialise slot metadata: {e}")))?;
        let tmp_path = self.metadata_path.with_extension("json.tmp");

        if let Some(parent) = self.metadata_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        if fs::exists(&tmp_path).await {
            fs::remove_dir_all(&tmp_path).await.ok();
            fs::remove_file(&tmp_path).await.ok();
        }

        tokio_fs::write(&tmp_path, payload)
            .await
            .map_err(|e| Error::internal(format!("failed to write slot metadata: {e}")))?;
        tokio_fs::rename(&tmp_path, &self.metadata_path)
            .await
            .map_err(|e| Error::internal(format!("failed to commit slot metadata: {e}")))?;

        Ok(())
    }

    async fn load_metadata(path: &Path) -> Result<SlotMetadata, Error> {
        if !fs::exists(path).await {
            return Ok(SlotMetadata::default());
        }

        let bytes = tokio_fs::read(path)
            .await
            .map_err(|e| Error::internal(format!("failed to read slot metadata: {e}")))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| Error::internal(format!("failed to parse slot metadata: {e}")))
    }
}

fn parse_state_marker(value: &str) -> Result<Option<Uuid>, Error> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Uuid::parse_str(trimmed)
        .map(Some)
        .map_err(|e| Error::internal(format!("invalid slot marker value: {e}")))
}

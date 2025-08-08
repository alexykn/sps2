//! Centralized, non-configurable filesystem paths for sps2
//!
//! These paths are deliberately not exposed via TOML configuration to keep the
//! installation prefix stable. Packages are built against this fixed prefix.

pub const PREFIX: &str = "/opt/pm";

pub const STORE_DIR: &str = "/opt/pm/store";
pub const STATES_DIR: &str = "/opt/pm/states";
pub const LIVE_DIR: &str = "/opt/pm/live";
pub const BIN_DIR: &str = "/opt/pm/live/bin";

pub const LOGS_DIR: &str = "/opt/pm/logs";
pub const KEYS_DIR: &str = "/opt/pm/keys";

pub const DB_PATH: &str = "/opt/pm/state.sqlite";

pub const LAST_GC_TIMESTAMP: &str = "/opt/pm/.last_gc_timestamp";

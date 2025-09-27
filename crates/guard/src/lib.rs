#![warn(mismatched_lifetime_syntaxes)]
//! Lightweight state guard utilities for verifying and healing package installations.

mod refcount;
mod store;
mod verifier;

pub use refcount::sync_refcounts_to_active_state;
pub use store::{StoreVerificationConfig, StoreVerificationStats, StoreVerifier};
pub use verifier::{Discrepancy, VerificationLevel, VerificationResult, Verifier};

//! Memoir core: everything that is not UI or a specific cloud provider.
//!
//! Remaining modules (store, sync, export) arrive in M2–M7; see
//! `docs/IMPLEMENTATION_PLAN.md`.

pub mod crypto;
pub mod db;
pub mod keystore;
pub mod vault;

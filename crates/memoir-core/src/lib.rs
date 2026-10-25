//! Memoir core: everything that is not UI or a specific cloud provider.
//!
//! Remaining modules (sync, export) arrive in M6–M7; see
//! `docs/IMPLEMENTATION_PLAN.md`.

pub mod blobs;
pub mod crypto;
pub mod db;
pub mod keystore;
pub mod store;
pub mod vault;

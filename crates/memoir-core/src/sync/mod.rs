//! Sync with a cloud [`StorageProvider`](crate::remote::StorageProvider).
//! See docs/ARCHITECTURE.md §6.

pub mod engine;
pub mod payload;

pub use engine::{Local, SyncConfig, SyncEngine, SyncError, SyncReport};
pub use payload::{Clock, Clocks, Merged, ObjectPayload, RevisionFile, merge};

//! Google Drive storage for Memoir: sign-in ([`oauth`]) and the
//! [`DriveProvider`] implementation of `memoir_core::remote::StorageProvider`.

pub mod drive;
pub mod oauth;

pub use drive::{Account, DriveConfig, DriveProvider, Folders, RetryPolicy};
pub use oauth::{OAuthConfig, OAuthError};

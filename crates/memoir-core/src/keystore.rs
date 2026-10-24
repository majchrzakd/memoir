//! Device-local secret storage: Windows Credential Manager, macOS Keychain.
//!
//! Used to remember the local database key on a trusted device, so the app
//! opens without asking for the passphrase. Turning on "require passphrase on
//! launch" simply means not storing it.

use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::crypto::SecretKey;

#[derive(Debug, thiserror::Error)]
pub enum KeystoreError {
    #[error("OS credential store: {0}")]
    Os(String),
    #[error("stored secret has the wrong size")]
    Corrupt,
}

pub trait SecretStore: Send + Sync {
    fn load(&self, account: &str) -> Result<Option<SecretKey>, KeystoreError>;
    fn store(&self, account: &str, key: &SecretKey) -> Result<(), KeystoreError>;
    /// Removing a missing entry is not an error.
    fn remove(&self, account: &str) -> Result<(), KeystoreError>;
}

/// Keychain account under which a vault's local database key is stored.
pub fn local_db_account(vault_id: &str) -> String {
    format!("local-db-key/{vault_id}")
}

/// The operating system's credential store.
pub struct OsKeychain {
    service: String,
}

impl OsKeychain {
    /// `service` is normally the app identifier, e.g. `com.memoir.notebook`.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring::Entry, KeystoreError> {
        keyring::Entry::new(&self.service, account).map_err(os_error)
    }
}

impl SecretStore for OsKeychain {
    fn load(&self, account: &str) -> Result<Option<SecretKey>, KeystoreError> {
        match self.entry(account)?.get_secret() {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                SecretKey::from_slice(&bytes)
                    .map(Some)
                    .ok_or(KeystoreError::Corrupt)
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(os_error(e)),
        }
    }

    fn store(&self, account: &str, key: &SecretKey) -> Result<(), KeystoreError> {
        self.entry(account)?
            .set_secret(key.as_bytes())
            .map_err(os_error)
    }

    fn remove(&self, account: &str) -> Result<(), KeystoreError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(os_error(e)),
        }
    }
}

fn os_error(e: keyring::Error) -> KeystoreError {
    KeystoreError::Os(e.to_string())
}

/// In-memory store for tests and for "don't remember on this device".
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, SecretKey>>);

impl SecretStore for MemoryStore {
    fn load(&self, account: &str) -> Result<Option<SecretKey>, KeystoreError> {
        Ok(self.0.lock().expect("lock").get(account).cloned())
    }

    fn store(&self, account: &str, key: &SecretKey) -> Result<(), KeystoreError> {
        self.0
            .lock()
            .expect("lock")
            .insert(account.to_owned(), key.clone());
        Ok(())
    }

    fn remove(&self, account: &str) -> Result<(), KeystoreError> {
        self.0.lock().expect("lock").remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::random_bytes;

    fn exercise(store: &dyn SecretStore) {
        let account = format!("test/{}", u64::from_le_bytes(random_bytes()));
        assert!(store.load(&account).unwrap().is_none());

        let key = SecretKey::generate();
        store.store(&account, &key).unwrap();
        assert_eq!(
            store.load(&account).unwrap().unwrap().as_bytes(),
            key.as_bytes()
        );

        let replaced = SecretKey::generate();
        store.store(&account, &replaced).unwrap();
        assert_eq!(
            store.load(&account).unwrap().unwrap().as_bytes(),
            replaced.as_bytes()
        );

        store.remove(&account).unwrap();
        assert!(store.load(&account).unwrap().is_none());
        store.remove(&account).unwrap();
    }

    #[test]
    fn memory_store() {
        exercise(&MemoryStore::default());
    }

    /// Touches the real OS credential store (entries are removed afterwards).
    #[test]
    #[cfg(any(windows, target_os = "macos"))]
    fn os_keychain() {
        exercise(&OsKeychain::new("com.memoir.notebook.tests"));
    }
}

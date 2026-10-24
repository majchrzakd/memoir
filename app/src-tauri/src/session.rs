//! Which vault this device has and whether it is unlocked.
//!
//! Files in the app's local data directory:
//! - `vault.json` — copy of the vault file (no secrets in the clear). Its
//!   presence means setup finished.
//! - `memoir.db` — the encrypted local database.
//!
//! On a trusted device the database key is remembered in the OS keychain,
//! so the app opens unlocked without asking for the passphrase.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use memoir_core::crypto::RecoveryKey;
use memoir_core::keystore::{SecretStore, local_db_account};
use memoir_core::store::{Store, StoreError};
use memoir_core::vault::{self, KdfParams, NewVault, UnlockedVault, VaultError, VaultFile};
use serde::Serialize;
use zeroize::Zeroizing;

const VAULT_FILE: &str = "vault.json";
const DB_FILE: &str = "memoir.db";
/// Trash older than this is deleted for good when the app opens.
const TRASH_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// No vault on this device yet.
    FirstRun,
    /// A vault was created; waiting for the user to confirm they saved the
    /// recovery key. Nothing is on disk yet.
    ConfirmRecoveryKey,
    Locked,
    Unlocked,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    RecoveryKey(#[from] memoir_core::crypto::recovery::RecoveryKeyError),
    #[error("could not access app data: {0}")]
    Io(#[from] std::io::Error),
    #[error("Memoir is locked")]
    Locked,
    #[error("not possible right now ({0})")]
    WrongState(&'static str),
}

pub type Result<T> = std::result::Result<T, SessionError>;

pub struct Session {
    dir: PathBuf,
    keystore: Box<dyn SecretStore>,
    kdf: KdfParams,
    pending: Option<NewVault>,
    store: Option<Store>,
}

impl Session {
    /// Opens the session and, if this device remembers the key, unlocks it.
    pub fn open(dir: PathBuf, keystore: Box<dyn SecretStore>) -> Self {
        Self::with_kdf(dir, keystore, KdfParams::DEFAULT)
    }

    fn with_kdf(dir: PathBuf, keystore: Box<dyn SecretStore>, kdf: KdfParams) -> Self {
        let mut session = Self {
            dir,
            keystore,
            kdf,
            pending: None,
            store: None,
        };
        if let Err(e) = session.try_auto_unlock() {
            log::warn!("automatic unlock failed: {e}");
        }
        session
    }

    pub fn status(&self) -> Status {
        if self.store.is_some() {
            Status::Unlocked
        } else if self.pending.is_some() {
            Status::ConfirmRecoveryKey
        } else if self.vault_path().exists() {
            Status::Locked
        } else {
            Status::FirstRun
        }
    }

    pub fn store(&mut self) -> Result<&mut Store> {
        self.store.as_mut().ok_or(SessionError::Locked)
    }

    /// First step of setup: creates the vault in memory and returns the
    /// recovery key to show. Nothing is written until [`Self::finish_setup`].
    pub fn begin_setup(&mut self, passphrase: &str) -> Result<Zeroizing<String>> {
        if !matches!(self.status(), Status::FirstRun | Status::ConfirmRecoveryKey) {
            return Err(SessionError::WrongState("a vault already exists"));
        }
        let new = vault::create(passphrase, self.kdf)?;
        let text = new.recovery_key.to_text();
        self.pending = Some(new);
        Ok(text)
    }

    /// Second step of setup, after the user confirmed the recovery key is saved.
    pub fn finish_setup(&mut self) -> Result<()> {
        let new = self
            .pending
            .take()
            .ok_or(SessionError::WrongState("setup was not started"))?;
        fs::create_dir_all(&self.dir)?;
        // A database without vault.json is from an interrupted setup and
        // can't be opened with the new key anyway.
        for leftover in [DB_FILE, "memoir.db-wal", "memoir.db-shm"] {
            let path = self.dir.join(leftover);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        let store = self.open_store(&new.file, &new.vault, true)?;
        // Written last: its presence marks setup as complete.
        write_atomically(&self.vault_path(), new.file.to_json().as_bytes())?;
        self.store = Some(store);
        Ok(())
    }

    pub fn unlock(&mut self, passphrase: &str, remember: bool) -> Result<()> {
        let file = self.read_vault_file()?;
        let vault = file.unlock(passphrase)?;
        self.store = Some(self.open_store(&file, &vault, remember)?);
        Ok(())
    }

    /// Unlocks with the recovery key and sets a new passphrase.
    pub fn unlock_with_recovery_key(
        &mut self,
        recovery_key: &str,
        new_passphrase: &str,
        remember: bool,
    ) -> Result<()> {
        let file = self.read_vault_file()?;
        let vault = file.unlock_with_recovery_key(&RecoveryKey::parse(recovery_key)?)?;
        let file = vault.change_passphrase(&file, new_passphrase, self.kdf)?;
        let store = self.open_store(&file, &vault, remember)?;
        write_atomically(&self.vault_path(), file.to_json().as_bytes())?;
        self.store = Some(store);
        Ok(())
    }

    fn try_auto_unlock(&mut self) -> Result<()> {
        if !self.vault_path().exists() {
            return Ok(());
        }
        let file = self.read_vault_file()?;
        let account = local_db_account(&file.vault_id);
        let Some(key) = self.keystore.load(&account).map_err(io_error)? else {
            return Ok(());
        };
        match Store::open(&self.db_path(), &key) {
            Ok(mut store) => {
                purge_old_trash(&mut store);
                self.store = Some(store);
                Ok(())
            }
            Err(e) => {
                // A stale key would fail on every launch; forget it.
                let _ = self.keystore.remove(&account);
                Err(e.into())
            }
        }
    }

    fn open_store(&self, file: &VaultFile, vault: &UnlockedVault, remember: bool) -> Result<Store> {
        let mut store = Store::open(&self.db_path(), &vault.keys().local_db)?;
        store.save_vault(file, vault)?;
        purge_old_trash(&mut store);
        let account = local_db_account(vault.vault_id());
        if remember {
            self.keystore
                .store(&account, &vault.keys().local_db)
                .map_err(io_error)?;
        } else {
            self.keystore.remove(&account).map_err(io_error)?;
        }
        Ok(store)
    }

    fn read_vault_file(&self) -> Result<VaultFile> {
        Ok(VaultFile::from_json(&fs::read_to_string(
            self.vault_path(),
        )?)?)
    }

    fn vault_path(&self) -> PathBuf {
        self.dir.join(VAULT_FILE)
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join(DB_FILE)
    }
}

fn purge_old_trash(store: &mut Store) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    if let Err(e) = store.purge_trashed_before(now - TRASH_RETENTION_MS) {
        log::warn!("could not empty old trash: {e}");
    }
}

fn io_error(e: memoir_core::keystore::KeystoreError) -> SessionError {
    SessionError::Io(std::io::Error::other(e.to_string()))
}

/// Writes via a temporary file and rename, so a crash never leaves a
/// half-written file behind.
fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use memoir_core::keystore::MemoryStore;
    use memoir_core::store::Placement;

    use super::*;

    const FAST: KdfParams = KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    const PASS: &str = "correct horse battery";

    /// Shares one in-memory keychain between sessions, like the OS one.
    struct SharedKeychain(Arc<MemoryStore>);

    impl SecretStore for SharedKeychain {
        fn load(
            &self,
            a: &str,
        ) -> std::result::Result<
            Option<memoir_core::crypto::SecretKey>,
            memoir_core::keystore::KeystoreError,
        > {
            self.0.load(a)
        }
        fn store(
            &self,
            a: &str,
            k: &memoir_core::crypto::SecretKey,
        ) -> std::result::Result<(), memoir_core::keystore::KeystoreError> {
            self.0.store(a, k)
        }
        fn remove(&self, a: &str) -> std::result::Result<(), memoir_core::keystore::KeystoreError> {
            self.0.remove(a)
        }
    }

    fn session(dir: &Path, keychain: &Arc<MemoryStore>) -> Session {
        Session::with_kdf(
            dir.to_owned(),
            Box::new(SharedKeychain(keychain.clone())),
            FAST,
        )
    }

    fn set_up(dir: &Path, keychain: &Arc<MemoryStore>) -> String {
        let mut s = session(dir, keychain);
        assert_eq!(s.status(), Status::FirstRun);
        let rk = s.begin_setup(PASS).unwrap().to_string();
        assert_eq!(s.status(), Status::ConfirmRecoveryKey);
        s.finish_setup().unwrap();
        assert_eq!(s.status(), Status::Unlocked);
        s.store()
            .unwrap()
            .create_note(Placement::last_in(None), "Hello", "body")
            .unwrap();
        rk
    }

    #[test]
    fn setup_then_reopen_unlocks_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        set_up(dir.path(), &keychain);

        let mut s = session(dir.path(), &keychain);
        assert_eq!(s.status(), Status::Unlocked);
        assert_eq!(s.store().unwrap().tree().unwrap()[0].title, "Hello");
    }

    #[test]
    fn abandoned_setup_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        let mut s = session(dir.path(), &keychain);
        s.begin_setup(PASS).unwrap();
        drop(s);
        assert_eq!(session(dir.path(), &keychain).status(), Status::FirstRun);
    }

    #[test]
    fn without_remembered_key_the_passphrase_is_needed() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        set_up(dir.path(), &keychain);

        let mut s = session(dir.path(), &keychain);
        s.unlock(PASS, false).unwrap(); // forget the key on this device

        let mut s = session(dir.path(), &keychain);
        assert_eq!(s.status(), Status::Locked);
        assert!(matches!(s.store(), Err(SessionError::Locked)));
        assert!(matches!(
            s.unlock("wrong passphrase", true),
            Err(SessionError::Vault(VaultError::WrongPassphrase))
        ));
        s.unlock(PASS, true).unwrap();
        assert_eq!(s.store().unwrap().tree().unwrap().len(), 1);
        assert_eq!(session(dir.path(), &keychain).status(), Status::Unlocked);
    }

    #[test]
    fn recovery_key_sets_a_new_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        let rk = set_up(dir.path(), &keychain);
        let keychain = Arc::new(MemoryStore::default()); // a "new device" keychain

        let mut s = session(dir.path(), &keychain);
        assert_eq!(s.status(), Status::Locked);
        s.unlock_with_recovery_key(&rk.to_lowercase(), "a new passphrase", false)
            .unwrap();
        assert_eq!(s.store().unwrap().tree().unwrap()[0].title, "Hello");

        let mut s = session(dir.path(), &keychain);
        assert!(s.unlock(PASS, false).is_err());
        s.unlock("a new passphrase", false).unwrap();
    }

    #[test]
    fn stale_keychain_entry_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        set_up(dir.path(), &keychain);
        let file = VaultFile::from_json(&fs::read_to_string(dir.path().join(VAULT_FILE)).unwrap())
            .unwrap();
        let account = local_db_account(&file.vault_id);
        keychain
            .store(&account, &memoir_core::crypto::SecretKey::generate())
            .unwrap();

        assert_eq!(session(dir.path(), &keychain).status(), Status::Locked);
        assert!(keychain.load(&account).unwrap().is_none());
    }
}

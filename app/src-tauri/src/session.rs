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

use memoir_core::blobs::{BlobError, BlobStore};
use memoir_core::crypto::{Keyring, RecoveryKey};
use memoir_core::keystore::{SecretStore, cloud_account, local_db_account};
use memoir_core::store::{Store, StoreError};
use memoir_core::vault::{self, KdfParams, NewVault, UnlockedVault, VaultError, VaultFile};
use serde::Serialize;
use zeroize::Zeroizing;

pub(crate) const VAULT_FILE: &str = "vault.json";
const DB_FILE: &str = "memoir.db";
/// The database with SQLite's write-ahead log files.
const DB_FILES: [&str; 3] = [DB_FILE, "memoir.db-wal", "memoir.db-shm"];
/// Encrypted attachments.
const BLOBS_DIR: &str = "blobs";
/// Decrypted copies of attachments opened in other programs; emptied on
/// every start and exit.
const OPENED_DIR: &str = "opened";
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
    Blob(#[from] BlobError),
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
    blobs: Option<BlobStore>,
    keys: Option<Keyring>,
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
            blobs: None,
            keys: None,
        };
        session.clear_opened_files();
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

    /// The database and the attachment files, both needed to add or read one.
    pub fn attachments(&mut self) -> Result<(&mut Store, &BlobStore)> {
        match (self.store.as_mut(), self.blobs.as_ref()) {
            (Some(store), Some(blobs)) => Ok((store, blobs)),
            _ => Err(SessionError::Locked),
        }
    }

    /// The unlocked vault's keys (for sync).
    pub fn keys(&self) -> Result<Keyring> {
        self.keys.clone().ok_or(SessionError::Locked)
    }

    /// The unlocked vault's public file (`vault.json`).
    pub fn vault_file(&self) -> Result<VaultFile> {
        if self.store.is_none() {
            return Err(SessionError::Locked);
        }
        self.read_vault_file()
    }

    /// The cloud sign-in (refresh token) remembered for this vault.
    pub fn cloud_token(&self) -> Result<Option<Zeroizing<String>>> {
        let account = cloud_account(&self.vault_file()?.vault_id);
        let Some(bytes) = self.keystore.load_bytes(&account).map_err(io_error)? else {
            return Ok(None);
        };
        Ok(Some(Zeroizing::new(
            String::from_utf8_lossy(&bytes).into_owned(),
        )))
    }

    pub fn set_cloud_token(&self, token: &str) -> Result<()> {
        let account = cloud_account(&self.vault_file()?.vault_id);
        self.keystore
            .store_bytes(&account, token.as_bytes())
            .map_err(io_error)
    }

    pub fn forget_cloud_token(&self) -> Result<()> {
        let account = cloud_account(&self.vault_file()?.vault_id);
        self.keystore.remove(&account).map_err(io_error)
    }

    /// Where decrypted copies go when an attachment is opened in another program.
    pub fn opened_dir(&self) -> PathBuf {
        self.dir.join(OPENED_DIR)
    }

    /// Deletes decrypted copies of opened attachments.
    pub fn clear_opened_files(&self) {
        let dir = self.opened_dir();
        if dir.exists()
            && let Err(e) = fs::remove_dir_all(&dir)
        {
            log::warn!("could not remove opened attachments: {e}");
        }
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
        self.remove_leftovers()?;
        let store = self.open_store(&new.file, &new.vault, true)?;
        // Written last: its presence marks setup as complete.
        write_atomically(&self.vault_path(), new.file.to_json().as_bytes())?;
        self.opened(store, new.vault.keys())?;
        Ok(())
    }

    /// Opens a notebook made on another device (its vault file, from the
    /// cloud) on this device, which has none yet. The notes arrive by sync.
    pub fn join(&mut self, file: &VaultFile, passphrase: &str, remember: bool) -> Result<()> {
        if !matches!(self.status(), Status::FirstRun | Status::ConfirmRecoveryKey) {
            return Err(SessionError::WrongState(
                "this device already has a notebook",
            ));
        }
        let vault = file.unlock(passphrase)?;
        self.pending = None;
        self.remove_leftovers()?;
        let store = self.open_store(file, &vault, remember)?;
        // Written last: its presence marks setup as complete.
        write_atomically(&self.vault_path(), file.to_json().as_bytes())?;
        self.opened(store, vault.keys())?;
        Ok(())
    }

    /// A database without vault.json is from an interrupted setup and can't
    /// be opened with a new key anyway.
    fn remove_leftovers(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        for leftover in DB_FILES {
            let path = self.dir.join(leftover);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    pub fn unlock(&mut self, passphrase: &str, remember: bool) -> Result<()> {
        let file = self.read_vault_file()?;
        let vault = file.unlock(passphrase)?;
        let store = self.open_store(&file, &vault, remember)?;
        self.opened(store, vault.keys())?;
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
        self.opened(store, vault.keys())?;
        Ok(())
    }

    /// Closes the notebook: the database, the keys and decrypted copies of
    /// opened attachments are gone until it's unlocked again, which needs
    /// the passphrase (or recovery key) even on a trusted device. Opening
    /// Memoir afresh still unlocks by itself there.
    pub fn lock(&mut self) {
        self.store = None;
        self.blobs = None;
        self.keys = None;
        self.pending = None;
        self.clear_opened_files();
    }

    /// Checks the current passphrase and sets a new one; the recovery key
    /// keeps working. Returns the updated vault file, for the cloud copy.
    pub fn change_passphrase(&mut self, current: &str, new: &str) -> Result<VaultFile> {
        let file = self.vault_file()?;
        let vault = file.unlock(current)?;
        let updated = vault.change_passphrase(&file, new, self.kdf)?;
        self.save_vault_file(&updated, &vault)?;
        Ok(updated)
    }

    /// Checks the passphrase and issues a new recovery key; the old one
    /// stops working. Returns the key to show once and the updated vault
    /// file, for the cloud copy.
    pub fn new_recovery_key(&mut self, passphrase: &str) -> Result<(Zeroizing<String>, VaultFile)> {
        let file = self.vault_file()?;
        let vault = file.unlock(passphrase)?;
        let (updated, recovery_key) = vault.replace_recovery_key(&file);
        self.save_vault_file(&updated, &vault)?;
        Ok((recovery_key.to_text(), updated))
    }

    /// Takes over a vault file from the cloud when it's a signed, newer
    /// version of this one (the passphrase or recovery key was changed on
    /// another device). Returns whether it did.
    pub fn adopt_vault_file(&mut self, remote: &VaultFile) -> Result<bool> {
        let local = self.vault_file()?;
        if !remote.supersedes(&local, &self.keys()?) {
            return Ok(false);
        }
        let (_, vault) = self
            .store()?
            .load_vault()?
            .ok_or(SessionError::WrongState("database has no vault"))?;
        self.save_vault_file(remote, &vault)?;
        Ok(true)
    }

    fn save_vault_file(&mut self, file: &VaultFile, vault: &UnlockedVault) -> Result<()> {
        self.store()?.save_vault(file, vault)?;
        write_atomically(&self.vault_path(), file.to_json().as_bytes())?;
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
                let (_, vault) = store
                    .load_vault()?
                    .ok_or(SessionError::WrongState("database has no vault"))?;
                self.opened(store, vault.keys())
            }
            Err(e) => {
                // A stale key would fail on every launch; forget it.
                let _ = self.keystore.remove(&account);
                Err(e.into())
            }
        }
    }

    /// Becomes unlocked: keeps the database and opens the attachment store.
    fn opened(&mut self, store: Store, keys: &Keyring) -> Result<()> {
        self.blobs = Some(BlobStore::open(self.dir.join(BLOBS_DIR), keys.clone())?);
        self.store = Some(store);
        self.keys = Some(keys.clone());
        Ok(())
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

/// Deletes the notebook in `dir` from this device: the database,
/// attachments and opened copies first, the vault file last, so a failure
/// part-way leaves a notebook that still opens (and can be deleted again).
/// Other files in `dir` stay.
pub(crate) fn delete_notebook_files(dir: &Path) -> std::io::Result<()> {
    let vault = dir.join(VAULT_FILE);
    let files = DB_FILES.map(|name| dir.join(name));
    let folders = [BLOBS_DIR, OPENED_DIR].map(|name| dir.join(name));
    for file in files {
        ignore_not_found(fs::remove_file(file))?;
    }
    for folder in folders {
        ignore_not_found(fs::remove_dir_all(folder))?;
    }
    ignore_not_found(fs::remove_file(vault.with_extension("tmp")))?;
    ignore_not_found(fs::remove_file(vault))
}

fn ignore_not_found(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Writes via a temporary file and rename, so a crash never leaves a
/// half-written file behind.
pub(crate) fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
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

    fn session(dir: &Path, keychain: &Arc<MemoryStore>) -> Session {
        Session::with_kdf(dir.to_owned(), Box::new(keychain.clone()), FAST)
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
    fn attachments_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        set_up(dir.path(), &keychain);
        let id = {
            let mut s = session(dir.path(), &keychain);
            let (store, blobs) = s.attachments().unwrap();
            let id = blobs.put(b"image bytes").unwrap();
            store.record_blob(&id, 11, "image/png").unwrap();
            id
        };

        let mut s = session(dir.path(), &keychain);
        let (store, blobs) = s.attachments().unwrap();
        assert_eq!(blobs.read(&id).unwrap(), b"image bytes");
        assert_eq!(store.blob(&id).unwrap().unwrap().mime, "image/png");

        // Locked: no access.
        let mut locked = session(dir.path(), &Arc::new(MemoryStore::default()));
        assert!(matches!(locked.attachments(), Err(SessionError::Locked)));
    }

    #[test]
    fn a_second_device_joins_with_the_vault_file_and_passphrase() {
        let first = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        set_up(first.path(), &keychain);
        let mut a = session(first.path(), &keychain);
        let file = a.vault_file().unwrap();

        let second = tempfile::tempdir().unwrap();
        let other_keychain = Arc::new(MemoryStore::default());
        let mut b = session(second.path(), &other_keychain);
        assert!(matches!(
            b.join(&file, "wrong passphrase", true),
            Err(SessionError::Vault(_))
        ));
        assert_eq!(b.status(), Status::FirstRun);

        b.join(&file, PASS, true).unwrap();
        assert_eq!(b.status(), Status::Unlocked);
        assert_eq!(b.vault_file().unwrap().vault_id, file.vault_id);
        assert!(
            b.store().unwrap().tree().unwrap().is_empty(),
            "notes come by sync"
        );
        let device_b = b.store().unwrap().device().to_owned();
        assert_ne!(device_b, a.store().unwrap().device());
        assert!(matches!(
            b.join(&file, PASS, true),
            Err(SessionError::WrongState(_))
        ));

        // Remembered: opens unlocked next time.
        drop(b);
        let mut b = session(second.path(), &other_keychain);
        assert_eq!(b.status(), Status::Unlocked);
        assert_eq!(b.store().unwrap().device(), device_b);
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
    fn locking_closes_the_notebook_until_the_passphrase_is_entered() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        set_up(dir.path(), &keychain); // remembered on this device
        let mut s = session(dir.path(), &keychain);
        assert_eq!(s.status(), Status::Unlocked);
        std::fs::create_dir_all(s.opened_dir()).unwrap();
        std::fs::write(s.opened_dir().join("photo.png"), b"decrypted").unwrap();

        s.lock();
        assert_eq!(s.status(), Status::Locked);
        assert!(matches!(s.store(), Err(SessionError::Locked)));
        assert!(matches!(s.keys(), Err(SessionError::Locked)));
        assert!(!s.opened_dir().exists(), "opened attachments removed");
        assert!(s.unlock("wrong passphrase", true).is_err());
        s.unlock(PASS, true).unwrap();
        assert_eq!(s.store().unwrap().tree().unwrap()[0].title, "Hello");
    }

    #[test]
    fn passphrase_and_recovery_key_can_be_changed() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(MemoryStore::default());
        let old_rk = set_up(dir.path(), &keychain);
        let mut s = session(dir.path(), &keychain);

        assert!(matches!(
            s.change_passphrase("not it", "a new passphrase"),
            Err(SessionError::Vault(_))
        ));
        s.change_passphrase(PASS, "a new passphrase").unwrap();
        let (new_rk, _) = s.new_recovery_key("a new passphrase").unwrap();

        let mut s = session(dir.path(), &Arc::new(MemoryStore::default()));
        assert!(s.unlock(PASS, false).is_err());
        s.unlock("a new passphrase", false).unwrap();
        s.lock();
        assert!(
            s.unlock_with_recovery_key(&old_rk, "another passphrase", false)
                .is_err()
        );
        s.unlock_with_recovery_key(&new_rk, "another passphrase", false)
            .unwrap();
    }

    #[test]
    fn a_change_on_one_device_is_adopted_by_another() {
        let a_dir = tempfile::tempdir().unwrap();
        let a_keys = Arc::new(MemoryStore::default());
        set_up(a_dir.path(), &a_keys);
        let mut a = session(a_dir.path(), &a_keys);
        let original = a.vault_file().unwrap();

        let b_dir = tempfile::tempdir().unwrap();
        let mut b = session(b_dir.path(), &Arc::new(MemoryStore::default()));
        b.join(&original, PASS, false).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));
        let changed = a.change_passphrase(PASS, "a new passphrase").unwrap();
        assert!(!b.adopt_vault_file(&original).unwrap(), "nothing new");
        let mut forged = changed.clone();
        forged.updated_at = Some(i64::MAX);
        assert!(!b.adopt_vault_file(&forged).unwrap(), "tampered");
        assert!(b.adopt_vault_file(&changed).unwrap());
        assert!(!b.adopt_vault_file(&original).unwrap(), "no rollback");

        b.lock();
        assert!(b.unlock(PASS, false).is_err());
        b.unlock("a new passphrase", false).unwrap();
        assert_eq!(b.vault_file().unwrap(), changed);
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

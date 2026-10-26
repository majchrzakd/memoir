//! `memoir decrypt`: a downloaded copy of the Memoir folder from Google Drive
//! plus the passphrase (or recovery key) → the notebook as Markdown files.
//!
//! It reads the folder exactly the way the app syncs from Drive (same
//! engine, same merge rules), into a temporary encrypted database that is
//! deleted afterwards, and then exports it like the app's "Export to
//! Markdown". Nothing in the folder is changed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use memoir_core::blobs::BlobStore;
use memoir_core::crypto::RecoveryKey;
use memoir_core::export::{ExportSummary, export_markdown};
use memoir_core::remote::FolderProvider;
use memoir_core::store::Store;
use memoir_core::sync::{Local, SyncConfig, SyncEngine, SyncError};
use memoir_core::vault::VaultFile;
use zeroize::Zeroizing;

const VAULT_FILE: &str = "vault.json";

pub enum Secret {
    Passphrase(Zeroizing<String>),
    RecoveryKey(Zeroizing<String>),
}

#[derive(Debug, thiserror::Error)]
pub enum DecryptError {
    #[error(
        "no vault.json in {0} — point at the Memoir folder downloaded from Google Drive \
         (the one with vault.json, objects and blobs in it)"
    )]
    NoVault(PathBuf),
    #[error("can't read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("wrong passphrase or recovery key, or a damaged vault.json ({0})")]
    Unlock(String),
    #[error("reading the notes failed: {0}")]
    Sync(#[from] SyncError),
    #[error("writing the Markdown files failed: {0}")]
    Export(#[from] memoir_core::export::ExportError),
    #[error("{0}")]
    Other(String),
}

pub struct Decrypted {
    pub summary: ExportSummary,
    /// Revision files read.
    pub revisions: usize,
}

/// The folder holding vault.json: `folder` itself, or a `Memoir` folder in it
/// (Google Drive downloads arrive as a zip with `Memoir/` inside).
pub fn find_vault_folder(folder: &Path) -> Result<PathBuf, DecryptError> {
    for candidate in [folder.to_owned(), folder.join("Memoir")] {
        if candidate.join(VAULT_FILE).is_file() {
            return Ok(candidate);
        }
    }
    Err(DecryptError::NoVault(folder.to_owned()))
}

struct TempLocal(Mutex<(Store, BlobStore)>);

impl Local for TempLocal {
    fn with<R>(&self, f: impl FnOnce(&mut Store, &BlobStore) -> R) -> Result<R, SyncError> {
        let mut guard = self.0.lock().map_err(|_| SyncError::Locked)?;
        let (store, blobs) = &mut *guard;
        Ok(f(store, blobs))
    }
}

pub async fn decrypt(folder: &Path, secret: Secret, out: &Path) -> Result<Decrypted, DecryptError> {
    let folder = find_vault_folder(folder)?;
    let vault_path = folder.join(VAULT_FILE);
    let json = std::fs::read_to_string(&vault_path).map_err(|source| DecryptError::Read {
        path: vault_path.clone(),
        source,
    })?;
    let file = VaultFile::from_json(&json).map_err(|e| DecryptError::Unlock(e.to_string()))?;
    let vault = match &secret {
        Secret::Passphrase(p) => file.unlock(p),
        Secret::RecoveryKey(k) => {
            let key = RecoveryKey::parse(k).map_err(|e| DecryptError::Unlock(e.to_string()))?;
            file.unlock_with_recovery_key(&key)
        }
    }
    .map_err(|e| DecryptError::Unlock(e.to_string()))?;
    let keys = vault.keys().clone();

    // A scratch notebook, encrypted like the app's, deleted when done.
    let scratch = tempfile::tempdir().map_err(|e| DecryptError::Other(e.to_string()))?;
    let store = Store::open(&scratch.path().join("memoir.db"), &keys.local_db)
        .map_err(|e| DecryptError::Other(e.to_string()))?;
    store
        .set_durable(false)
        .map_err(|e| DecryptError::Other(e.to_string()))?;
    let blobs = BlobStore::open(scratch.path().join("blobs"), keys.clone())
        .map_err(|e| DecryptError::Other(e.to_string()))?;
    let local = TempLocal(Mutex::new((store, blobs)));

    let engine = SyncEngine::new(
        Arc::new(FolderProvider::new(&folder)),
        keys,
        SyncConfig::default(),
    );
    let report = engine.pull_only(&local).await?;

    let guard = local
        .0
        .lock()
        .map_err(|_| DecryptError::Other("lock".into()))?;
    let (store, blobs) = &*guard;
    let summary = export_markdown(store, blobs, out)?;
    Ok(Decrypted {
        summary,
        revisions: report.downloaded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use memoir_core::crypto::Keyring;
    use memoir_core::store::Placement;
    use memoir_core::vault::{self, KdfParams};

    const PASS: &str = "correct horse battery";

    /// A device that syncs into `cloud` (a folder like the Drive one).
    fn device(dir: &Path, keys: &Keyring) -> TempLocal {
        std::fs::create_dir_all(dir).unwrap();
        let store = Store::open(&dir.join("memoir.db"), &keys.local_db).unwrap();
        let blobs = BlobStore::open(dir.join("blobs"), keys.clone()).unwrap();
        TempLocal(Mutex::new((store, blobs)))
    }

    #[tokio::test]
    async fn a_drive_folder_copy_and_the_recovery_key_are_enough() {
        let tmp = tempfile::tempdir().unwrap();
        let cloud = tmp.path().join("download").join("Memoir");
        std::fs::create_dir_all(&cloud).unwrap();
        let new = vault::create(
            PASS,
            KdfParams {
                m_kib: 64,
                t: 1,
                p: 1,
            },
        )
        .unwrap();
        std::fs::write(cloud.join("vault.json"), new.file.to_json()).unwrap();
        let keys = new.vault.keys().clone();

        // Two devices write to the "Drive" folder, concurrently.
        let a = device(&tmp.path().join("a"), &keys);
        let b = device(&tmp.path().join("b"), &keys);
        let engine = SyncEngine::new(
            Arc::new(FolderProvider::new(&cloud)),
            keys.clone(),
            SyncConfig::default(),
        );
        let (note, img) = a
            .with(|s, blobs| {
                let folder = s.create_folder(Placement::last_in(None), "Trips").unwrap();
                let img = blobs.put(b"jpeg").unwrap();
                s.record_blob(&img, 4, "image/jpeg").unwrap();
                let note = s
                    .create_note(
                        Placement::last_in(Some(folder)),
                        "Rome",
                        &format!("Day 1\n\n![photo](attachment:{img})\n"),
                    )
                    .unwrap();
                (note, img)
            })
            .unwrap();
        engine.sync(&a).await.unwrap();
        engine.sync(&b).await.unwrap();
        let edited = format!("Day 1 from A\n\n![photo](attachment:{img})\n");
        a.with(|s, _| s.save_note(&note, "Rome", &edited).unwrap())
            .unwrap();
        b.with(|s, _| s.add_reply(&note, "Reply from B").unwrap())
            .unwrap();
        engine.sync(&a).await.unwrap();
        engine.sync(&b).await.unwrap();

        let out = tmp.path().join("notes");
        let recovery = Zeroizing::new(new.recovery_key.to_text().to_string());
        // Pointing at the folder that contains Memoir/ works too.
        let done = decrypt(
            &tmp.path().join("download"),
            Secret::RecoveryKey(recovery),
            &out,
        )
        .await
        .unwrap();
        assert_eq!(
            (
                done.summary.notes,
                done.summary.folders,
                done.summary.attachments
            ),
            (1, 1, 1)
        );
        let text = std::fs::read_to_string(out.join("Trips").join("Rome.md")).unwrap();
        assert!(text.contains("Day 1 from A"), "{text}");
        assert!(text.contains("Reply from B"), "{text}");
        assert!(text.contains("](../attachments/photo.jpg)"), "{text}");
        assert_eq!(
            std::fs::read(out.join("attachments").join("photo.jpg")).unwrap(),
            b"jpeg"
        );

        // The passphrase works as well; a wrong one doesn't.
        let out2 = tmp.path().join("notes2");
        decrypt(
            &cloud,
            Secret::Passphrase(Zeroizing::new(PASS.into())),
            &out2,
        )
        .await
        .unwrap();
        assert!(matches!(
            decrypt(
                &cloud,
                Secret::Passphrase(Zeroizing::new("nope".into())),
                &tmp.path().join("x")
            )
            .await,
            Err(DecryptError::Unlock(_))
        ));
        assert!(matches!(
            decrypt(
                tmp.path(),
                Secret::Passphrase(Zeroizing::new(PASS.into())),
                &out2
            )
            .await,
            Err(DecryptError::NoVault(_))
        ));
    }
}

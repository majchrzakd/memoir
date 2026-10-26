//! Sync timings for a large notebook against the in-memory cloud (so this
//! measures Memoir's own work, not the network):
//! `cargo test -p memoir-core --release --test sync_bench -- --ignored --nocapture`

use std::sync::{Arc, Mutex};
use std::time::Instant;

use memoir_core::blobs::BlobStore;
use memoir_core::crypto::{Keyring, SecretKey};
use memoir_core::remote::MemoryProvider;
use memoir_core::store::{Placement, Store};
use memoir_core::sync::{Local, SyncConfig, SyncEngine, SyncError};

struct Device(Mutex<(Store, BlobStore)>);

impl Local for Device {
    fn with<R>(&self, f: impl FnOnce(&mut Store, &BlobStore) -> R) -> Result<R, SyncError> {
        let mut guard = self.0.lock().unwrap();
        let (store, blobs) = &mut *guard;
        Ok(f(store, blobs))
    }
}

fn device(dir: &std::path::Path, keys: &Keyring) -> Device {
    std::fs::create_dir_all(dir).unwrap();
    let store = Store::open(&dir.join("memoir.db"), &keys.local_db).unwrap();
    let blobs = BlobStore::open(dir.join("blobs"), keys.clone()).unwrap();
    Device(Mutex::new((store, blobs)))
}

#[test]
#[ignore = "benchmark"]
fn ten_thousand_notes() {
    const NOTES: usize = 10_000;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let keys = Keyring::derive(&SecretKey::from_bytes([1; 32]), 1);
        let provider = Arc::new(MemoryProvider::new());
        let engine = SyncEngine::new(provider.clone(), keys.clone(), SyncConfig::default());

        let a = device(&tmp.path().join("a"), &keys);
        let body = "Some text in a note, a few lines long.\n".repeat(25); // ~1 KB
        a.with(|s, _| {
            let mut folders = Vec::new();
            for i in 0..100 {
                folders.push(
                    s.create_folder(Placement::last_in(None), &format!("Folder {i}"))
                        .unwrap(),
                );
            }
            for i in 0..NOTES {
                s.create_note(
                    Placement::last_in(Some(folders[i % 100])),
                    &format!("Note {i}"),
                    &body,
                )
                .unwrap();
            }
        })
        .unwrap();

        let t = Instant::now();
        let r = engine.sync(&a).await.unwrap();
        println!(
            "first sync, uploading {} objects: {:?}",
            r.uploaded,
            t.elapsed()
        );

        let t = Instant::now();
        engine.sync(&a).await.unwrap();
        println!("quiet round: {:?}", t.elapsed());

        let b = device(&tmp.path().join("b"), &keys);
        let t = Instant::now();
        let r = engine.sync(&b).await.unwrap();
        println!(
            "new device, downloading {} revisions: {:?}",
            r.downloaded,
            t.elapsed()
        );

        a.with(|s, _| {
            let id = s
                .tree()
                .unwrap()
                .into_iter()
                .find(|i| i.title == "Note 5")
                .unwrap()
                .id;
            s.save_note(&id, "Note 5", "edited\n").unwrap();
        })
        .unwrap();
        let t = Instant::now();
        engine.sync(&a).await.unwrap();
        engine.sync(&b).await.unwrap();
        println!("one edit, pushed and pulled: {:?}", t.elapsed());
    });
}

//! Multi-device simulation of sync.
//!
//! Three devices share one cloud (an in-memory provider). A seeded random
//! script makes them create, edit, rename, move, trash, restore and delete
//! notes, folders and replies, and sync at random moments while the network
//! fails at random — including in the middle of a sync. Afterwards the
//! network recovers and everyone syncs until quiet. Then:
//!
//! 1. all devices hold exactly the same notebook (convergence), and
//! 2. every word typed anywhere still exists in some note, unless that note
//!    was deleted for good (no lost text).
//!
//! More seeds / steps: `MEMOIR_SIM_SEEDS=500 MEMOIR_SIM_STEPS=200 cargo test -p memoir-core --test sync_sim -- --nocapture`

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use memoir_core::blobs::BlobStore;
use memoir_core::crypto::{Keyring, SecretKey};
use memoir_core::remote::{MemoryProvider, ProviderError};
use memoir_core::store::{Kind, ObjectId, Placement, Store};
use memoir_core::sync::{Local, ObjectPayload, SyncConfig, SyncEngine, SyncError};

const DEVICES: usize = 3;

thread_local! {
    static TRACE: bool = std::env::var("MEMOIR_SIM_TRACE").is_ok();
    /// A word to follow through the run (debugging).
    static WATCH: Option<String> = std::env::var("MEMOIR_SIM_WATCH").ok();
}

struct Device {
    state: Mutex<(Store, BlobStore)>,
    _dir: tempfile::TempDir,
}

impl Local for Device {
    fn with<R>(&self, f: impl FnOnce(&mut Store, &BlobStore) -> R) -> Result<R, SyncError> {
        let mut guard = self.state.lock().unwrap();
        let (store, blobs) = &mut *guard;
        Ok(f(store, blobs))
    }
}

impl Device {
    fn new(keys: &Keyring) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("memoir.db"), &keys.local_db).unwrap();
        store.set_durable(false).unwrap(); // speed: the simulation doesn't test power loss
        let blobs = BlobStore::open(dir.path().join("blobs"), keys.clone()).unwrap();
        Self {
            state: Mutex::new((store, blobs)),
            _dir: dir,
        }
    }

    fn store<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        f(&mut self.state.lock().unwrap().0)
    }

    /// Everything on the device, comparable across devices.
    fn snapshot(&self) -> BTreeMap<ObjectId, ObjectPayload> {
        self.store(|s| {
            s.all_object_ids()
                .unwrap()
                .into_iter()
                .map(|id| (id, s.payload(&id).unwrap().unwrap()))
                .collect()
        })
    }
}

/// xorshift64*: small, deterministic.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<T: Clone>(&mut self, items: &[T]) -> Option<T> {
        (!items.is_empty()).then(|| items[self.below(items.len())].clone())
    }
}

struct Sim {
    rng: Rng,
    provider: Arc<MemoryProvider>,
    engine: SyncEngine<MemoryProvider>,
    devices: Vec<Device>,
    /// token → object it was typed into
    typed: Vec<(String, ObjectId)>,
    /// words that were on screen when someone deleted their note for good
    /// (those may be gone; anything else must survive)
    deleted_words: HashSet<String>,
    next_token: usize,
    keys: Keyring,
    /// Unreferenced attachments are cleaned up at once in this run.
    eager_blob_cleanup: bool,
    conflicts: usize,
    /// What happened, for failure reports.
    events: Vec<String>,
}

impl Sim {
    fn new(seed: u64) -> Self {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let keys = Keyring::derive(&SecretKey::from_bytes([7; 32]), 1);
        let provider = Arc::new(MemoryProvider::new());
        let clock = Arc::new(AtomicI64::new(1_000_000));
        // Some runs clean up superseded revisions at once (merges then often
        // lack a common ancestor), others keep them.
        let eager_blob_cleanup = rng.chance(20);
        let config = SyncConfig {
            revision_grace_ms: if rng.chance(50) { 0 } else { i64::MAX / 4 },
            blob_grace_ms: if eager_blob_cleanup { 0 } else { i64::MAX / 4 },
            full_list_every_ms: if rng.chance(50) { 0 } else { i64::MAX / 4 },
            cleanup_every_ms: 0,
        };
        let engine = SyncEngine::new(provider.clone(), keys.clone(), config)
            .with_clock(move || clock.fetch_add(1000, Ordering::SeqCst));
        Self {
            rng,
            provider,
            engine,
            devices: (0..DEVICES).map(|_| Device::new(&keys)).collect(),
            typed: Vec::new(),
            deleted_words: HashSet::new(),
            next_token: 0,
            keys,
            eager_blob_cleanup,
            conflicts: 0,
            events: Vec::new(),
        }
    }

    fn log(&mut self, d: usize, what: String) {
        if TRACE.with(|t| *t) {
            eprintln!("[{d}] {what}");
        }
        self.events
            .push(format!("#{} [{d}] {what}", self.events.len()));
    }

    fn token(&mut self) -> String {
        self.next_token += 1;
        format!("tok{}", self.next_token)
    }

    async fn step(&mut self) {
        let d = self.rng.below(DEVICES);
        let roll = self.rng.below(100);
        let (folders, notes, trashed) = self.devices[d].store(|s| {
            let tree = s.tree().unwrap();
            let folders: Vec<ObjectId> = tree
                .iter()
                .filter(|i| i.kind == Kind::Folder)
                .map(|i| i.id)
                .collect();
            let notes: Vec<ObjectId> = tree
                .iter()
                .filter(|i| i.kind == Kind::Note)
                .map(|i| i.id)
                .collect();
            let trashed: Vec<ObjectId> = s.trashed_items().unwrap().iter().map(|i| i.id).collect();
            (folders, notes, trashed)
        });
        let somewhere = |rng: &mut Rng| -> Option<ObjectId> {
            if rng.chance(30) {
                None
            } else {
                rng.pick(&folders)
            }
        };

        match roll {
            // Sync, often with a network failure somewhere in the round.
            0..=24 => {
                if self.rng.chance(30) {
                    let skip = self.rng.below(12);
                    self.provider
                        .fail_after(skip, ProviderError::Network("flaky".into()));
                }
                match self.engine.sync(&self.devices[d]).await {
                    Ok(r) => {
                        self.conflicts += r.conflicts;
                        self.log(
                            d,
                            format!(
                                "sync: up {} down {} changed {:?}",
                                r.uploaded, r.downloaded, r.changed
                            ),
                        );
                    }
                    Err(e) if e.is_transient() => self.log(d, format!("sync failed: {e}")),
                    Err(e) => panic!("sync failed: {e}"),
                }
            }
            25..=34 => {
                let token = self.token();
                let parent = somewhere(&mut self.rng);
                let id = self.devices[d].store(|s| {
                    s.create_note(Placement::last_in(parent), "note", &format!("{token}\n"))
                });
                if let Ok(id) = id {
                    self.log(d, format!("create note {id} in {parent:?} with {token}"));
                    self.typed.push((token, id));
                }
            }
            35..=39 => {
                let parent = somewhere(&mut self.rng);
                if let Ok(id) =
                    self.devices[d].store(|s| s.create_folder(Placement::last_in(parent), "folder"))
                {
                    self.log(d, format!("create folder {id} in {parent:?}"));
                }
            }
            40..=59 => {
                // Insert a word on a random line of a note or reply.
                let target = if self.rng.chance(25) {
                    let note = self.rng.pick(&notes);
                    let replies = note.map(|n| self.devices[d].store(|s| s.replies(&n).unwrap()));
                    replies.and_then(|r| self.rng.pick(&r.iter().map(|r| r.id).collect::<Vec<_>>()))
                } else {
                    self.rng.pick(&notes)
                };
                if let Some(id) = target {
                    let token = self.token();
                    let note = self.devices[d].store(|s| s.note(&id)).unwrap();
                    let mut lines: Vec<String> = note.body.lines().map(str::to_owned).collect();
                    let at = self.rng.below(lines.len() + 1);
                    if self.rng.chance(15) {
                        // An attachment, on the same line as the word.
                        let data = format!("file for {token}").repeat(1 + self.rng.below(50));
                        let blob = self.devices[d]
                            .with(|s, blobs| {
                                let id = blobs.put(data.as_bytes()).unwrap();
                                s.record_blob(&id, data.len() as u64, "text/plain").unwrap();
                                id
                            })
                            .unwrap();
                        self.log(d, format!("attach {} for {token}", &blob[..12]));
                        lines.insert(at, format!("{token} [file](attachment:{blob})"));
                    } else {
                        lines.insert(at, token.clone());
                    }
                    let body = lines.join("\n") + "\n";
                    if self.devices[d]
                        .store(|s| s.save_note(&id, &note.title, &body))
                        .is_ok()
                    {
                        self.log(d, format!("type {token} into {id}"));
                        self.typed.push((token, id));
                    }
                }
            }
            60..=63 => {
                if let Some(note) = self.rng.pick(&notes) {
                    let token = self.token();
                    if let Ok(id) =
                        self.devices[d].store(|s| s.add_reply(&note, &format!("{token}\n")))
                    {
                        self.log(d, format!("reply {id} to {note} with {token}"));
                        self.typed.push((token, id));
                    }
                }
            }
            64..=69 => {
                let all: Vec<ObjectId> = folders.iter().chain(&notes).copied().collect();
                if let Some(id) = self.rng.pick(&all) {
                    let title = format!("title{}", self.rng.below(1000));
                    let _ = self.devices[d].store(|s| s.rename(&id, &title));
                    self.log(d, format!("rename {id}"));
                }
            }
            70..=79 => {
                let all: Vec<ObjectId> = folders.iter().chain(&notes).copied().collect();
                if let Some(id) = self.rng.pick(&all) {
                    let parent = somewhere(&mut self.rng);
                    let r = self.devices[d].store(|s| s.move_item(&id, Placement::last_in(parent)));
                    self.log(d, format!("move {id} to {parent:?}: {:?}", r.err()));
                }
            }
            80..=85 => {
                let all: Vec<ObjectId> = folders.iter().chain(&notes).copied().collect();
                if let Some(id) = self.rng.pick(&all) {
                    let _ = self.devices[d].store(|s| s.move_to_trash(&id));
                    self.log(d, format!("trash {id}"));
                }
            }
            86..=89 => {
                if let Some(id) = self.rng.pick(&trashed) {
                    let _ = self.devices[d].store(|s| s.restore(&id));
                    self.log(d, format!("restore {id}"));
                }
            }
            90..=92 => {
                if let Some(id) = self.rng.pick(&trashed) {
                    let before = self.devices[d].snapshot();
                    if self.devices[d].store(|s| s.purge(&id)).is_ok() {
                        self.log(d, format!("purge {id}"));
                        self.forget_deleted(d, &before);
                    }
                }
            }
            93..=95 => {
                if let Some(note) = self.rng.pick(&notes) {
                    let replies = self.devices[d].store(|s| s.replies(&note).unwrap());
                    if let Some(r) = self.rng.pick(&replies) {
                        let before = self.devices[d].snapshot();
                        if self.devices[d].store(|s| s.delete_reply(&r.id)).is_ok() {
                            self.log(d, format!("delete reply {}", r.id));
                            self.forget_deleted(d, &before);
                        }
                    }
                }
            }
            _ => {
                if let Some(id) = self.rng.pick(&notes) {
                    let later = self.rng.chance(70);
                    let _ = self.devices[d].store(|s| s.set_later(&id, later, None));
                    self.log(d, format!("later {id} {later}"));
                }
            }
        }
    }

    /// Words in objects that device `d` just deleted for good.
    fn forget_deleted(&mut self, d: usize, before: &BTreeMap<ObjectId, ObjectPayload>) {
        let after = self.devices[d].snapshot();
        for (id, p) in before {
            if !after.contains_key(id) {
                self.log(d, format!("  (deleted {id}: {:?})", p.body));
                self.deleted_words
                    .extend(p.body.split_whitespace().map(str::to_owned));
            }
        }
    }

    /// A new device (empty) syncs and ends up with the same notebook, without
    /// changing anything in the cloud.
    async fn join_new_device(&mut self, seed: u64) {
        let fresh = Device::new(&self.keys);
        let report = self.engine.sync(&fresh).await.unwrap();
        assert_eq!(
            report.uploaded, 0,
            "seed {seed}: a new device uploaded something"
        );
        let again = self.engine.sync(&fresh).await.unwrap();
        assert_eq!(
            (again.uploaded, again.downloaded),
            (0, 0),
            "seed {seed}: not settled"
        );
        assert!(
            fresh.snapshot() == self.devices[0].snapshot(),
            "seed {seed}: new device differs"
        );
        self.devices.push(fresh);
        self.settle().await;
        self.check(seed);
    }

    /// Network recovers; everyone syncs until nothing moves.
    async fn settle(&mut self) {
        self.provider.clear_failures();
        for round in 0..12 {
            let mut moved = false;
            for d in &self.devices {
                let report = self.engine.sync(d).await.unwrap();
                self.conflicts += report.conflicts;
                moved |= report.uploaded > 0 || report.downloaded > 0;
            }
            if !moved && round > 0 {
                return;
            }
        }
        panic!("sync did not settle within 12 rounds");
    }

    fn check(&self, seed: u64) {
        let first = self.devices[0].snapshot();
        for (i, d) in self.devices.iter().enumerate().skip(1) {
            let other = d.snapshot();
            if other != first {
                let only: Vec<_> = first.keys().filter(|k| !other.contains_key(k)).collect();
                let extra: Vec<_> = other.keys().filter(|k| !first.contains_key(k)).collect();
                let differing: Vec<_> = first
                    .iter()
                    .filter(|(k, v)| other.get(k).is_some_and(|o| o != *v))
                    .map(|(k, v)| (k, v, &other[k]))
                    .take(2)
                    .collect();
                for id in only.iter().chain(&extra) {
                    self.dump(id);
                }
                for d in &self.devices {
                    d.store(|s| eprintln!("pending: {:?}", s.outbox().unwrap()));
                }
                panic!(
                    "seed {seed}: device 0 and {i} differ\nonly on 0: {only:?}\nonly on {i}: {extra:?}\nfirst differences: {differing:#?}"
                );
            }
        }
        for d in &self.devices {
            assert_eq!(
                d.store(|s| s.pending_changes().unwrap()),
                0,
                "seed {seed}: unsynced changes left"
            );
        }

        // Everything is reachable: no object points at a missing parent.
        for p in first.values() {
            if let Some(parent) = p.parent_id
                && !first.contains_key(&parent)
            {
                self.dump(&p.id);
                panic!(
                    "seed {seed}: {} is orphaned (parent {parent} is gone)",
                    p.id
                );
            }
        }

        // Every attachment a note refers to is on every device.
        if !self.eager_blob_cleanup {
            for (i, d) in self.devices.iter().enumerate() {
                d.with(|s, blobs| {
                    for p in s.all_object_ids().unwrap().iter().filter_map(|id| s.payload(id).unwrap()) {
                        for a in &p.attachments {
                            if !blobs.contains(&a.id) {
                                for e in self.events.iter().filter(|e| e.contains(&a.id[..12]) || e.contains(&p.id.to_string())) {
                                    eprintln!("{e}");
                                }
                                for (j, o) in self.devices.iter().enumerate() {
                                    if j != i {
                                        o.with(|os, ob| eprintln!("device {j}: has {} remote {:?}", ob.contains(&a.id), os.remote_blob(&a.id))).unwrap();
                                    }
                                }
                                let in_notes = s.blobs_in_notes().unwrap().contains(&a.id);
                                panic!("seed {seed}: attachment {} of {} missing on device {i} (remote {:?}, in notes {in_notes})", a.id, p.id, s.remote_blob(&a.id));
                            }
                            assert!(blobs.read(&a.id).is_ok(), "seed {seed}: attachment {} unreadable", a.id);
                        }
                    }
                }).unwrap();
            }
        }

        let text: String = first
            .values()
            .map(|p| p.body.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let words: HashSet<&str> = text.split_whitespace().collect();
        for (token, object) in &self.typed {
            if !words.contains(token.as_str()) && !self.deleted_words.contains(token) {
                self.dump(object);
                panic!("seed {seed}: {token} (typed into {object}) was lost");
            }
        }
    }
}

impl Sim {
    fn watch(&self) {
        let Some(word) = WATCH.with(Clone::clone) else {
            return;
        };
        for (i, d) in self.devices.iter().enumerate() {
            d.store(|s| {
                for id in s.all_object_ids().unwrap() {
                    let p = s.payload(&id).unwrap().unwrap();
                    if p.body.split_whitespace().any(|w| w == word) {
                        let sync = s.sync_object(&id).unwrap();
                        eprintln!(
                            "   [{i}] {id} parent {:?} trashed {:?} pending {:?} heads {:?}",
                            p.parent_id,
                            p.trashed_at,
                            s.outbox_version(&id).unwrap(),
                            sync.heads
                        );
                    }
                }
            });
        }
    }

    fn dump(&self, id: &ObjectId) {
        let key = id.to_string();
        for e in self.events.iter().filter(|e| e.contains(&key)) {
            eprintln!("{e}");
        }
        for (i, d) in self.devices.iter().enumerate() {
            d.store(|s| {
                eprintln!(
                    "== device {i} ({}): now {:?}",
                    s.device(),
                    s.payload(id).unwrap()
                );
                for r in s.revisions_of(id).unwrap() {
                    eprintln!(
                        "   rev {} parents {:?}
      {:?}",
                        r.rev_id, r.parents, r.payload
                    );
                }
            });
        }
    }
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn devices_converge_without_losing_text() {
    let first = env("MEMOIR_SIM_FIRST", 0);
    let seeds = env("MEMOIR_SIM_SEEDS", 30);
    let steps = env("MEMOIR_SIM_STEPS", 80);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut stats = (0usize, 0usize, 0usize);
    for seed in first..first + seeds {
        runtime.block_on(async {
            let mut sim = Sim::new(seed);
            for _ in 0..steps {
                sim.step().await;
                sim.watch();
            }
            sim.settle().await;
            sim.check(seed);
            sim.join_new_device(seed).await;
            stats.0 += sim.devices[0].snapshot().len();
            stats.1 += sim.typed.len();
            stats.2 += sim.conflicts;
        });
    }
    println!(
        "{seeds} runs × {steps} steps: {} objects, {} words typed, {} conflict copies — all converged",
        stats.0, stats.1, stats.2
    );
}

/// The Memoir folder deleted in Drive (all files gone): the next full
/// listing uploads everything again, attachments included.
#[test]
fn a_lost_cloud_copy_is_rebuilt() {
    use memoir_core::remote::{Area, StorageProvider};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let keys = Keyring::derive(&SecretKey::from_bytes([9; 32]), 1);
        let provider = Arc::new(MemoryProvider::new());
        let config = SyncConfig {
            full_list_every_ms: 0,
            ..SyncConfig::default()
        };
        let engine = SyncEngine::new(provider.clone(), keys.clone(), config);
        let device = Device::new(&keys);
        device.store(|s| {
            let folder = s.create_folder(Placement::last_in(None), "F").unwrap();
            s.create_note(Placement::last_in(Some(folder)), "N", "text\n")
                .unwrap();
        });
        let blob = device.state.lock().unwrap().1.put(b"file").unwrap();
        device.store(|s| {
            s.record_blob(&blob, 4, "text/plain").unwrap();
            s.create_note(
                Placement::last_in(None),
                "A",
                &format!("[f](attachment:{blob})"),
            )
            .unwrap();
        });
        engine.sync(&device).await.unwrap();

        for area in [Area::Objects, Area::Blobs] {
            for f in provider.list(area).await.unwrap() {
                provider.delete(&f.id).await.unwrap();
            }
        }
        let report = engine.sync(&device).await.unwrap();
        assert_eq!(report.uploaded, 3, "every object again");
        assert_eq!(
            provider.list(Area::Blobs).await.unwrap().len(),
            1,
            "and the attachment"
        );

        let fresh = Device::new(&keys);
        engine.sync(&fresh).await.unwrap();
        assert!(fresh.snapshot() == device.snapshot());
        assert!(fresh.state.lock().unwrap().1.contains(&blob));
    });
}

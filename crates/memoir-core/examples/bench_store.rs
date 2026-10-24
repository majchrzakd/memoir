//! Store performance against the targets in docs/ARCHITECTURE.md.
//!
//! ```powershell
//! cargo run -p memoir-core --release --example bench_store
//! ```
//!
//! Builds a 10 000-note vault (110 folders, notes of 0.5–4 KB of Markdown)
//! in a temporary directory, then measures cold open, tree loading, opening
//! notes, saving and searching.

use std::time::{Duration, Instant};

use memoir_core::crypto::SecretKey;
use memoir_core::store::{ObjectId, Placement, Store};

const NOTES: usize = 10_000;
const TOP_FOLDERS: usize = 10;
const SUBFOLDERS: usize = 10;

const WORDS: &[&str] = &[
    "meeting",
    "project",
    "budget",
    "travel",
    "kraków",
    "recipe",
    "garden",
    "invoice",
    "family",
    "holiday",
    "deadline",
    "review",
    "design",
    "rust",
    "notebook",
    "idea",
    "book",
    "music",
    "doctor",
    "car",
    "insurance",
    "house",
    "renovation",
    "zażółć",
    "gęślą",
    "jaźń",
    "spotkanie",
    "zakupy",
    "pomysł",
    "lista",
    "the",
    "and",
    "with",
    "for",
    "about",
    "tomorrow",
    "weekend",
];

/// Small deterministic PRNG (xorshift64*) so runs are comparable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn word(&mut self) -> &'static str {
        WORDS[self.below(WORDS.len())]
    }
}

fn markdown(rng: &mut Rng, unique: usize) -> String {
    let target = 500 + rng.below(3500);
    let mut md = format!("# Note {unique}\n\nToken uniq{unique} here.\n\n");
    while md.len() < target {
        match rng.below(6) {
            0 => md.push_str(&format!("## {} {}\n\n", rng.word(), rng.word())),
            1 => md.push_str(&format!(
                "- [ ] {} {}\n- [x] {}\n\n",
                rng.word(),
                rng.word(),
                rng.word()
            )),
            2 => md.push_str(&format!(
                "[{}](https://example.com/{})\n\n",
                rng.word(),
                rng.below(1000)
            )),
            _ => {
                for _ in 0..(10 + rng.below(30)) {
                    md.push_str(rng.word());
                    md.push(' ');
                }
                md.push_str("\n\n");
            }
        }
    }
    md
}

fn stats(label: &str, target_ms: Option<f64>, mut samples: Vec<Duration>) {
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let median = ms(samples[samples.len() / 2]);
    let p99 = ms(samples[(samples.len() * 99 / 100).min(samples.len() - 1)]);
    let verdict = match target_ms {
        Some(t) if p99 <= t => format!("OK   (target {t} ms)"),
        Some(t) => format!("SLOW (target {t} ms)"),
        None => String::new(),
    };
    println!("{label:<34} median {median:>8.2} ms   p99 {p99:>8.2} ms   {verdict}");
}

fn time<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

fn main() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("bench.db");
    let key = SecretKey::from_bytes([9; 32]);
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);

    // ---- Build ----
    let started = Instant::now();
    let mut store = Store::open(&path, &key).expect("open");
    let mut folders = Vec::new();
    for t in 0..TOP_FOLDERS {
        let top = store
            .create_folder(Placement::last_in(None), &format!("Folder {t}"))
            .unwrap();
        folders.push(top);
        for s in 0..SUBFOLDERS {
            folders.push(
                store
                    .create_folder(Placement::last_in(Some(top)), &format!("Sub {t}.{s}"))
                    .unwrap(),
            );
        }
    }
    let mut notes: Vec<ObjectId> = Vec::with_capacity(NOTES);
    let mut bytes = 0;
    for i in 0..NOTES {
        let body = markdown(&mut rng, i);
        bytes += body.len();
        let parent = folders[rng.below(folders.len())];
        notes.push(
            store
                .create_note(
                    Placement::last_in(Some(parent)),
                    &format!("{} {i}", rng.word()),
                    &body,
                )
                .unwrap(),
        );
    }
    drop(store);
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!(
        "built {NOTES} notes ({:.1} MB Markdown, DB {:.1} MB) in {:.1} s\n",
        bytes as f64 / 1e6,
        size as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );

    // ---- Cold open + tree: what the app does at startup ----
    let mut startup = Vec::new();
    for _ in 0..5 {
        let (_, d) = time(|| {
            let store = Store::open(&path, &key).unwrap();
            store.tree().unwrap()
        });
        startup.push(d);
    }
    stats("open database + load tree", Some(300.0), startup);

    let mut store = Store::open(&path, &key).unwrap();
    let tree_samples = (0..20).map(|_| time(|| store.tree().unwrap()).1).collect();
    stats("load tree (110 folders, 10k notes)", None, tree_samples);

    // ---- Open note ----
    let open_samples = (0..1000)
        .map(|_| {
            let id = notes[rng.below(notes.len())];
            time(|| store.note(&id).unwrap()).1
        })
        .collect();
    stats("open note", Some(50.0), open_samples);

    // ---- Save note (durable: fsync per commit) ----
    let save_samples = (0..200)
        .map(|i| {
            let id = notes[rng.below(notes.len())];
            let mut body = store.note(&id).unwrap().body;
            body.push_str(&format!("\nedit {i} {}", rng.word()));
            time(|| store.save_note(&id, "edited", &body).unwrap()).1
        })
        .collect();
    stats("save note", None, save_samples);

    // ---- Search ----
    for query in [
        "uniq4242",
        "kraków",
        "gesla",
        "renov",
        "meeting budget",
        "zz-nothing",
    ] {
        let mut hits = 0;
        let samples = (0..50)
            .map(|_| {
                let (h, d) = time(|| store.search(query, 50).unwrap());
                hits = h.len();
                d
            })
            .collect();
        stats(
            &format!("search {query:?} ({hits} hits)"),
            Some(30.0),
            samples,
        );
    }
}

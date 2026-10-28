# Memoir — Architecture

Personal, end-to-end encrypted, offline-first notebook. Notes live on the
user's own Google Drive; nobody but the user (not Google, not us) can read them.

Status: **agreed design, pre-implementation** (2026-10-24).

---

## 1. Decisions

| Topic | Decision |
|---|---|
| Platforms | **Windows first**, then macOS, then iOS |
| Cloud | **Google Drive** only (iCloud deferred; it is unusable on Windows) |
| Privacy | **End-to-end encryption is mandatory**, from v1 |
| Model | **Offline-first**: local DB is the source of truth, sync runs in background |
| Shutdown | App **flushes pending changes to the cloud before exiting** |
| Structure | **Folder hierarchy** (arbitrary depth) in a left tree pane, note editor in the right pane |
| Stack | **Rust core + Tauri 2**, UI in **Svelte 5 + TypeScript** |
| Editor | **WYSIWYG, Evernote-like** (TipTap/ProseMirror); expect several iterations |
| Out of scope | Collaboration, sharing, web clipper, OCR, reminders, AI, templates, own backend |

### Why Svelte 5 (not SolidJS)
Both are fast and small. Svelte reads closest to plain HTML/JS, which matters
since the owner isn't a frontend specialist; it has the larger ecosystem and an
official Tauri template. The UI layer is thin — all logic lives in Rust — so
this choice is low-risk and reversible.

### Performance targets
| Metric | Target |
|---|---|
| Cold start → usable | < 300 ms (Windows desktop) |
| Open note | < 50 ms |
| Full-text search, 10k notes | < 30 ms |
| Typing | never blocked by disk, crypto or network |
| Data loss | none — not on crash, kill, power loss, or sync conflict |

---

## 2. High-level structure

```
┌─────────────────────────────────────────────────────────────┐
│ UI  (Svelte 5 + TS, Vite)                                   │
│   Tree pane (virtualized, drag&drop) │ Editor pane (TipTap) │
│   Search · Sync status indicator · Unlock screen            │
├──────────────── Tauri 2 IPC (typed via tauri-specta) ───────┤
│ app/src-tauri — thin shell: commands, window lifecycle,     │
│                 close-time sync flush, OS keychain          │
├─────────────────────────────────────────────────────────────┤
│ memoir-core (pure Rust, no Tauri dependency, fully testable)│
│   vault   — key hierarchy, unlock, recovery key             │
│   crypto  — envelope format, XChaCha20-Poly1305, zstd       │
│   store   — SQLCipher DB: tree, notes, FTS5, outbox         │
│   sync    — revision DAG, push/pull, merge, GC              │
│   export  — decrypted Markdown export                       │
│   import  — Evernote (.enex) import                         │
├─────────────────────────────────────────────────────────────┤
│ memoir-gdrive — StorageProvider impl (OAuth PKCE, REST v3)  │
│ memoir-cli    — offline recovery/export tool                │
└─────────────────────────────────────────────────────────────┘
```

Repository layout:

```
Cargo.toml              # workspace
crates/
  memoir-core/
  memoir-gdrive/
  memoir-cli/
app/
  src-tauri/            # Tauri 2 Rust shell
  src/                  # Svelte UI
docs/
```

---

## 3. Data model

Two kinds of synced **objects**, both identified by a random 128-bit ID (ULID):

```
Folder { id, parent_id: Option<Id>, name, sort_key, trashed_at, updated_at }
Note   { id, parent_id: Option<Id>, title, body_md, sort_key, tags,
         created_at, updated_at, trashed_at, attachments: [BlobId] }
```

- **Hierarchy** = `parent_id` + `sort_key` (fractional index string). Moving or
  reordering an item changes one field on one object → small, merge-friendly.
- Notes appear as leaves in the tree (folders first, then notes, by `sort_key`).
- **Note body is stored as Markdown (GFM)**. Reasons: line-based 3-way merge
  works well on it, it makes export trivial, and it naturally limits the editor
  to features worth having. The editor is WYSIWYG; Markdown is only the storage
  format.
- **Trash** is a state (`trashed_at`); purged after 30 days.
- **Blobs** (images, files) are immutable, identified by
  `HMAC-SHA256(blob_id_key, plaintext)` — dedup without leaking content
  equality to anyone without the key.

---

## 4. End-to-end encryption

### Key hierarchy
```
passphrase ──Argon2id──► KEK ──unwraps──► Master Key (random 256-bit)
recovery key (random, shown once) ──────► unwraps the same Master Key

Master Key ──HKDF──┬─► object_key    (notes, folders)
                   ├─► blob_key      (attachments)
                   ├─► blob_id_key   (HMAC for blob IDs)
                   └─► local_db_key  (SQLCipher)
```
- `vault.json` on Drive holds only: Argon2id salt + params, Master Key wrapped
  by KEK, Master Key wrapped by recovery key, format version. Useless without
  the passphrase or recovery key.
- New device: sign in to Google → download `vault.json` → enter passphrase.
- Changing the passphrase only re-wraps the Master Key (instant).
- **If both passphrase and recovery key are lost, data is unrecoverable.**
  This is inherent to real E2EE and must be stated clearly at setup.
- Day-to-day: `local_db_key` is cached in the OS keychain (Windows Credential
  Manager / macOS & iOS Keychain), so the app opens instantly. Optional setting:
  "require passphrase on launch".
- The master key itself is kept inside the encrypted local database (from M2),
  so background sync works without the passphrase once the database is open.
  The keychain therefore holds a single 32-byte secret per vault.

### Envelope format (every file uploaded to Drive)
```
magic "MMR1" | version | kind | key epoch | nonce (24 B) | ciphertext+tag
AAD = header bytes + object id + revision id
plaintext = length | zstd(serialized object) | zero padding to 256-byte multiple
```
Byte-level specification: [FORMAT.md](FORMAT.md).
- XChaCha20-Poly1305 (random 192-bit nonces — safe with no nonce bookkeeping).
- AAD binds the ciphertext to its identity, so files can't be swapped or
  replayed under another name undetected.
- Large blobs use chunked streaming AEAD (STREAM construction), so memory stays
  flat and corruption is detected per chunk.

### What Google can still see
File count, sizes, timestamps, and access times. **Not**: titles, folder
names, structure, content, tags, attachment types. Accepted trade-off.

### Local data at rest
The local DB is **SQLCipher** (encrypted SQLite); local blob cache is stored
encrypted in the same envelope format. A stolen laptop disk reveals nothing
without the keychain entry / passphrase.

### Locking, passphrase and recovery key changes
- **Lock** (sidebar button, Ctrl/⌘+L, Mac menu, or automatically after N idle
  minutes, Settings → Security): the database closes and the keys leave
  memory (`Session::lock`); decrypted copies of opened attachments are
  deleted; the UI drops what it showed. Unlocking needs the passphrase or
  recovery key, even on a device that remembers the key — that only skips
  the passphrase when Memoir starts.
- **Change passphrase / new recovery key** (Settings → Security, both after
  checking the current passphrase) re-wrap the master key, so nothing is
  re-encrypted. The new `vault.json` is stamped (`updated_at`) and signed
  with a key derived from the master key (`mac`, see FORMAT.md) and
  uploaded; the copy in Drive is replaced.
- **Other devices** compare their `vault.json` with Drive's at the start of
  a session, after unlocking and after such a change
  (`cloud::reconcile_vault`). They adopt Drive's copy only if it's signed by
  the master key and newer, so a copy planted or rolled back in Drive is
  ignored. Until a device has synced once, it still opens with the old
  passphrase (and the old recovery key still works there).

---

## 5. Local storage

Single SQLCipher database (WAL mode) — transactional, crash-safe, fast:

| Table | Purpose |
|---|---|
| `meta` | device id, local copy of `vault.json`, master key |
| `objects` | folders and notes: tree position, title, tags, timestamps (small rows) |
| `note_bodies` | note Markdown, kept apart so loading the tree doesn't read it |
| `notes_fts` | FTS5 index over title + plain text of body + tags |
| `outbox` | objects changed locally and not yet pushed, one row each; a missing object = deletion |
| `history` | earlier versions of notes (see below) |
| `revisions` *(M6)* | per object: last synced revision id + **base content** (for 3-way merge) |
| `remote_state` *(M6)* | Drive `changes` page token, folder ids, etc. |

**Trash**: trashing a folder marks only the folder; everything inside is
hidden with it and comes back with it. Restoring an item whose folder is gone
or trashed puts it at the top level. Items whose parent is missing (possible
after sync) are shown at the top level.

**Note history**: a save keeps the version it replaces when that version sat
unchanged for 10 minutes (an edit after a pause), or when the newest kept
version is 10 minutes older (a checkpoint during long continuous editing).
Autosaves in between are not kept; at most 50 versions per note. Saving
unchanged content is a no-op, so it never creates sync traffic.

**Edited time** (`updated_at`): the last change to a note's *content*
(title, body, tags). Moving, trashing/restoring and the Later flag don't
change it; they are still queued for sync, which tracks them separately (M6).

**Later list**: a note can be marked "Later" (`later_at`) with an optional
due date (`due_date`, a calendar date `YYYY-MM-DD`, no time zone). The Later
tab lists such notes, dated ones first (soonest first), then by creation
time, newest first (same order within one due date).

**Threads**: a note can have replies, like a Slack thread. A reply is a
note without a title whose parent is a note — so trash, purge, search,
history and sync treat replies like any other object. Rules (enforced in the
store): only notes have replies, one level only, replies don't appear in the
tree (the note shows a reply count), can't be moved, trashed on their own or
put on Later, and are deleted directly (no trash). Replies are ordered by
creation time. A search match in a reply points at its note (`reply_id`).
The UI shows the thread below the note or in a side column (a switch,
remembered per device; side needs a wide window). Unsent reply drafts live
in memory only.

**Links between notes**: a note links to another as `[text](note:<id>)`
in its Markdown (`NOTE_LINK_SCHEME`), so links sync, merge and survive
history like any other text, and keep working when the target is renamed
or moved. In the editor, typing `[[` (or the toolbar's link button) opens a
picker of note titles (`editor/NoteLinkDialog.svelte`); with text selected
it links that text, otherwise it inserts the note's title. Clicking a note
link opens the note; one to a deleted or trashed note says so (and is shown
struck through). Below the text, **Linked from** lists the notes whose text
or thread links here (`Store::backlinks`: a scan of the note text, like
search, with no index to keep in step with edits and sync; trashed notes
are left out). Export turns note links into relative links to the `.md`
files; links to notes that are gone stay as they were. Links never cross
workspaces.

**Attachments** (M4): pasted, dropped or attached files are encrypted into
`blobs/<id>.mmr` in the app's data folder (blob stream format; the id is a
keyed HMAC of the content, so identical files are stored once and the id is
bound into the encryption). The `blobs` table keeps size, type and — for
sync — whether a blob is uploaded. Notes refer to them in Markdown:
`![name](attachment:<id>)` for images, `[name](attachment:<id>)` for other
files (shown as chips). Images are served to the webview by the `attachment`
URI scheme (`http://attachment.localhost/<id>` on Windows), decrypted in
memory per request with `Cache-Control: no-store`, so the webview's disk cache
never holds plaintext; only `image/*` attachments are served. Opening a file
in another program writes a decrypted copy to `opened/`, which is emptied on
every start and exit; "Save as…" writes where the user chooses. Limit: 100 MB
per file. Unreferenced attachments are not yet cleaned up (M6, with sync).

**Order of siblings**: fractional-index sort keys
(`crates/memoir-core/src/store/sort_key.rs`), so a move or reorder changes a
single object.

**Measured** (M2, `cargo run -p memoir-core --release --example bench_store`,
10 000 notes / 23.5 MB Markdown / 110 folders, Windows desktop):

| Operation | median | p99 | target |
|---|---|---|---|
| open database + load tree | 15 ms | 20 ms | 300 ms |
| open note | 0.02 ms | 0.06 ms | 50 ms |
| save note (fsync) | 1.7 ms | 19 ms | — |
| search, rare word | 0.1 ms | 0.6 ms | 30 ms |
| search, word in most notes (top 50) | 18–23 ms | 25 ms | 30 ms |

Every edit: editor → debounce 300 ms → `save_note` command → one transaction
(update `objects` + FTS + `outbox`). After commit, the change survives a crash.

The DB is **never** placed in a cloud-synced folder.

---

## 6. Sync engine

### The core problem
Google Drive API v3 has **no compare-and-swap**: it ignores `If-Match` on
updates (verified in the M0 spike). Overwriting a file in place can silently lose an edit made on another
device at nearly the same time. So:

### Append-only revision files
Each upload **creates a new immutable file**; nothing is overwritten.

```
Memoir/                         (visible Drive folder, scope drive.file)
  vault.json
  objects/<objectId>.<revId>.mmr   ← one file per object revision
  blobs/<blobId>.mmr               ← immutable attachments
```

Each revision file (encrypted, context `object:<id>:<rev>`) holds
`{v, parents, object}`: the whole object (folder, note or reply) plus the
ids of the revisions it replaces — a tiny DAG per object, like git. The
code is in `crates/memoir-core/src/sync/` (engine, merge rules) and
`store/sync_state.rs` (bookkeeping tables from migration 5).

A sync round (`SyncEngine::sync`) is safe to interrupt anywhere; every step
is idempotent and the next round continues:

1. **Pull.** Follow the change feed from the stored cursor (a full listing
   on first sync and once a day, which also catches anything the feed
   missed). Index new files, download and decrypt new revisions.
2. **Merge.** For each object with new revisions: the *heads* (revisions no
   other revision replaces) are merged pairwise against their lowest common
   ancestor, always in the same order, so every device computes the same
   result. Unpushed local changes are then merged in against what they were
   based on. A new revision is queued only if no head already says the same
   (otherwise two devices would ping-pong merges forever).
3. **Attachments.** Download the ones notes refer to but this device lacks.
4. **Push.** For each outbox entry: upload its attachments first (so a
   revision never refers to a missing file), then a new revision whose
   parents are the heads it was based on.
5. **Clean up** (hourly): revisions replaced more than 48 h ago are deleted
   (ancestors first, so an interrupted clean-up never leaves an old revision
   whose successors are gone); attachments nothing refers to any more — no
   note, reply, history version or kept revision — are deleted after 7 days,
   and only while nothing local is unsynced.

### Merge rules
- Every field change is stamped with a clock (time + device id) in
  `objects.clocks`. Title, position (parent + sort key), tags, trash state and
  Later state: the latest change wins, per field. A save records only the
  fields it actually changed, so a body edit on one device never undoes a
  rename on another.
- Body: 3-way text merge (`diffy`). If both sides changed the same lines, the
  newer text stays in the note and the other is saved next to it as
  "‹title› (conflict copy)" (a reply's copy is another reply). Its id is
  derived from the losing text, so two devices resolving the same conflict
  create one copy, not two. **Text is never silently dropped.**
- Deleted for good vs. edited: an edit the deleting device never saw brings
  the object back. "Never saw" means it differs from the common ancestor (by
  content — a merge can bring in an older edit without moving any clock),
  falling back to clocks when the ancestor was already cleaned up.
- A revision whose file is gone from the cloud was cleaned up, which only
  happens to replaced ones; it never counts as a head, even on a device that
  never saw its replacement (e.g. one offline for longer than 48 h).
- Tree anomalies, resolved the same way on every device:
  - move cycle (A into B on one device, B into A on another): the folder moved
    last goes to the top level;
  - parent deleted elsewhere while a child was edited (so the child came
    back): the child moves to the top level.

### Editing while sync runs
The editor sends each save together with the version it started from
(`save_note` → `Store::save_note_from`). If sync changed the note in the
meantime, the store merges instead of overwriting and returns the result,
which the editor shows (unless more typing is pending — that save merges it).
Notes and replies changed by sync are reloaded via the `sync-changed` event
when they have no unsaved edits.

### When sync runs (app/src-tauri/src/sync.rs)
- On startup/unlock, after connecting Drive, on window focus (at most every
  15 s), 3 s after edits pause (at most 30 s after the first unsynced edit),
  every 60 s, and on demand ("Sync now", or clicking the status).
- One round at a time. Failures wait for the next minute; the provider
  already retries transient errors with backoff.
- **On close**: the window hides, the editor saves, and pending changes are
  pushed (≤ 15 s). If offline or timed out they stay durable in the outbox
  and go up on next launch.
- The sidebar shows the status: synced / syncing / N to sync / offline /
  sync problem (sign in again, Drive full).
- Not yet: a Windows shutdown-block reason (M8).

### A second device
First-run screen → "Open your notebook from Google Drive": Google sign-in,
`vault.json` is downloaded from the Memoir folder, unlocked with the
passphrase, and a new local database is created (new device id). The notes
then arrive with the first sync.

### Testing the sync engine
`crates/memoir-core/tests/sync_sim.rs`: three devices (each with its own
encrypted database and attachment folder) share an in-memory provider. A
seeded random script creates, edits (inserting a unique word on a random
line), attaches files, renames, moves, trashes, restores, deletes for good,
replies and syncs, while the network fails at random points — also in the
middle of rounds. Half the runs clean up replaced revisions at once (merges
then often lack a common ancestor). Then the network recovers and everyone
syncs until quiet, and the test checks:

- all devices hold exactly the same notebook, with nothing left unsynced;
- every word typed is still in some note, unless the user deleted the note
  it was in while that word was on screen;
- nothing points at a missing parent, and every attachment a note refers to
  is on every device;
- a brand-new device that syncs ends up identical, without uploading anything.

CI runs 30 runs × 80 steps; more with
`MEMOIR_SIM_SEEDS=1000 MEMOIR_SIM_STEPS=150` (`MEMOIR_SIM_FIRST` picks the
first seed, `MEMOIR_SIM_TRACE=1` prints every step). Bugs it found and fixed
during M6: attachments from other devices failed to save on Windows
(flushing a read-only handle), deletes winning over edits they never saw (three
variants), and old revisions resurfacing after clean-up.

---

## 7. Google Drive integration

- OAuth 2.0 **Authorization Code + PKCE**, loopback redirect on desktop,
  custom URL scheme on iOS.
- Scope **`drive.file`** — the app only sees files it created; non-sensitive
  scope, no Google security review needed.
- Google Cloud project must be set to **"In production"** publishing status:
  in "Testing" status refresh tokens expire after 7 days.
- Refresh token stored in the OS keychain.
- Visible `Memoir/` folder (not hidden `appDataFolder`) so the user can see and
  back up their (encrypted) data.
- Bonus: Drive keeps its own file revisions, but our append-only DAG already
  gives us history independent of it.

**Implementation (M5)**: `memoir_core::remote::StorageProvider` is the
interface the sync engine uses — immutable files in three areas (`Memoir/`,
`objects/`, `blobs/`), list, download, upload (always a new file), delete,
and a change feed with a cursor; `MemoryProvider` is an in-memory version with
failure injection for tests. `memoir-gdrive` implements it for Drive:
- requests carry an access token refreshed a minute before expiry, and once
  more on a 401; an `invalid_grant` refresh means "connect again";
- network errors, 429, 5xx and rate-limit 403s are retried with exponential
  backoff and jitter (honouring `Retry-After`), up to 5 tries;
- uploads over 5 MB use a resumable session in 8 MiB chunks; after a failure
  the session is asked how much arrived and the upload continues from there
  (an expired session is restarted);
- the `Memoir` folders are found (oldest wins if two devices raced) or created.

Connecting (Settings) runs the browser sign-in, sets up the folders, and
uploads `vault.json` — or refuses a Drive whose `vault.json` belongs to a
different notebook (joining an existing notebook is part of M6). The refresh
token is kept in the OS keychain per vault (`cloud-token/<vault id>`); the
account and folder ids in the `remote_state` table. Disconnecting revokes the
token and forgets both; files in Drive stay. The OAuth client comes from
`MEMOIR_GOOGLE_CLIENT_ID` / `_SECRET` (build time, or run time in development).

**Checked against a real Drive** (`drive_check`, 2026-10-25): sign-in,
folders (found, not duplicated, on a second run), a 27 B and a 12 MB
(resumable) upload in ~4 s, listing, byte-exact downloads, and the change feed
all work. The change feed is **eventually consistent**: new files appeared at
once, deletions about 0.8 s later. M6 therefore treats the feed as a hint
that catches up — a change missing from one poll arrives in a later one — and
never assumes a just-made change is already visible to other devices.

---

### Workspaces
A device can hold several notebooks, e.g. a personal one in a personal
Google Drive and a work one in a work Drive (`app/src-tauri/src/workspaces.rs`).
Each workspace is a complete, separate notebook: its own vault (passphrase,
recovery key), database, attachments and Google sign-in — nothing is shared,
so the work notebook can't be read with the personal passphrase and vice
versa. Keychain entries are per vault id, so they never collide.

- **Files:** `workspaces.json` in the data folder lists the workspaces
  (id, name — nothing secret) and which is open. The first workspace is the
  data folder itself, so a notebook from before workspaces simply becomes it
  (and its UI preferences keep their names); others live in
  `workspaces/<id>/` with the same layout.
- **One open at a time.** Switching (the name at the top of the sidebar, the
  lock screen, or Settings → Workspaces) saves the editor, pushes pending
  changes (≤ 10 s), waits for a running sync round, closes the notebook like
  a lock does, and opens the other one — unlocked at once if this device
  remembers its key, otherwise at the unlock screen. Changes that didn't
  make it up stay in that workspace's outbox and go up when it's open
  again. Only the open workspace syncs.
- **Adding** one asks for a name and shows the first-run screen: a new
  notebook, or one already in (another account's) Google Drive; Cancel
  removes it again. Google's sign-in always offers the account chooser
  (`prompt=consent select_account`), since the browser is often signed in
  to several. A Drive whose notebook is already a workspace on this device
  is refused, as is (as before) connecting a Drive that holds a different
  notebook — one notebook per Google account.
- **Removing** one (not the open one) deletes its local files and keychain
  entries; its Drive copy stays, so it can be opened again from there.
- Expanded folders and the open note are remembered per workspace.

## 8. Data ownership & recovery

- **Export** (Settings → Export to Markdown; `memoir_core::export`): a new
  folder with a directory per folder and a `.md` file per note. Each note has
  YAML front matter (title, created, updated, tags, Later/due, trashed) and
  its replies appended under "Replies". Attachments go to `attachments/`,
  linked by relative paths (named after the link text, with an extension
  from the type); the trash goes to `Trash/`. File names are made safe for
  Windows/macOS/Linux and de-duplicated; note files keep their edit time.
  The export is plain text by design, and the app says so.
- **Import from Evernote** (Settings → Import from Evernote; `memoir_core::import`):
  the user picks one or more `.enex` files (Evernote's notebook export); each
  becomes a top-level folder named after the file. The file is streamed note
  by note (quick-xml), so large exports don't load into memory, and the
  notebook is locked only one note at a time, so the app and sync keep
  working meanwhile. A note keeps its title, created/updated dates and tags.
  ENML becomes the editor's own Markdown dialect, exactly as the editor would
  save it (a vitest round-trips samples to check): headings, bold/italic/
  strike/code, links, lists and both kinds of Evernote checklists, tables,
  code blocks, quotes and rules; anything else keeps its text. Resources
  become attachments, matched to `en-media` by MD5; images show inline, other
  files as links, and resources the content never references are appended.
  Evernote-encrypted text and attachments over the 100 MB limit can't be
  imported; the note says where they were and the summary counts them.
- **`memoir decrypt <folder> --out <dir> [--recovery-key]`** (`crates/memoir-cli`):
  given a downloaded copy of the `Memoir/` Drive folder (or the folder the
  Drive zip was extracted into) and the passphrase or recovery key, it reads
  the folder with the same sync engine the app uses (`FolderProvider`,
  `SyncEngine::pull_only`, which never writes to the folder) into a
  temporary encrypted database. It then exports it exactly like the app. A
  test has two devices write such a folder concurrently and recovers it with
  the recovery key. The file format is in `docs/FORMAT.md`.
- **History** (History button on a note): versions kept on this device —
  after a 10-minute pause and every 10 minutes of continuous editing, the
  last 50 — including text replaced by sync, so another device's change can
  be undone. Restoring keeps the current text as a version too. History is
  not synced.

---

## 9. Key libraries

| Area | Crate / package |
|---|---|
| Async runtime | `tokio` |
| DB | `rusqlite` (`bundled-sqlcipher-vendored-openssl`), FTS5 |
| Crypto | `chacha20poly1305`, `argon2`, `hkdf`, `hmac`, `sha2`, `zeroize`, `rand` |
| Compression | `zstd` |
| Merge | `diffy` |
| IDs / ordering | `ulid`, fractional indexing (small own module) |
| HTTP / OAuth | `reqwest` (rustls), `oauth2` |
| Keychain | `keyring` |
| Serialization | `serde`, `postcard` (objects), `serde_json` (vault.json) |
| IPC types | hand-written TS mirror in `app/src/lib/api/types.ts` (tauri-specta skipped: still pre-1.0, and ~20 commands are easy to keep in sync) |
| Testing | `proptest`, `insta` |
| UI | Svelte 5, Vite, TypeScript, TipTap 3 (+ Markdown serializer) |

---

## 10. Desktop app (M3)

**Files** in the app's local data directory (`%LOCALAPPDATA%\com.memoir.notebook`):
`vault.json` (public copy; its presence means setup finished), `memoir.db`
(SQLCipher), `logs/`.

**Session states** (`app/src-tauri/src/session.rs`): first run → (recovery
key shown, nothing on disk until the user confirms they saved it) →
unlocked; locked ⇄ unlocked. On launch the database key is read from the OS
keychain and the app opens unlocked; without it (or with "Remember on this
device" off) the passphrase — or the recovery key plus a new passphrase — is
required. A stale keychain entry is discarded.

**UI** (`app/src`): Svelte 5. `lib/app.svelte.ts` holds state and actions;
`lib/api` is the typed backend. Tree rows are virtualized (only visible rows
are in the DOM). Each opened note gets a fresh TipTap editor (undo history
never crosses notes); autosave runs 300 ms after the last keystroke and saves
are serialized, so an older save can never overwrite a newer one.

**Closing**: the backend emits `flush-editor`; the UI saves pending edits and
answers with `editor_flushed` (≤ 2 s) before the window hides and sync runs.

**Development without the desktop shell**: `npm run dev` in a normal browser
uses an in-memory mock backend (`lib/api/mock.ts`, sample notes;
`?mock=first_run` / `?mock=locked` show the other screens).

## 11. Hardening (M8)

- **Logs** (`app/src-tauri/src/logging.rs`): `memoir.log` next to the data
  (`%LOCALAPPDATA%\com.memoir.notebook\logs`), 5 × 1 MB, local times;
  crashes (panics) are recorded. One line per sync round with counts and
  timings. Rule: never note content (titles, text, file names), only ids,
  counts, timings and error kinds. Settings → About opens the folder.
- **Recovery paths:**
  - An unreadable revision file is skipped (older versions are used).
  - A Memoir folder deleted or trashed in Drive is recreated, and its
    contents uploaded again: once per session the app checks the folders
    and `vault.json`. After any full listing, objects with no file left in
    the cloud are queued (`Store::enqueue_lost`).
  - Revoked access shows "Sync problem → Connect again".
  - A wiped keychain means unlocking with the passphrase and connecting
    Drive again.
  - Images not on this device yet show a placeholder and are retried after
    each sync.
- **Windows shutdown** with unsynced changes (`shutdown_block.rs`): on
  `WM_QUERYENDSESSION` Memoir registers a shutdown-block reason, runs the
  closing sequence (save, push ≤ 15 s, exit) and lets shutdown continue.
- **Performance** (10,000 notes, in-memory cloud, `tests/sync_bench.rs`):
  first sync 19 s, a quiet round 0.27 s, a new device 2 s, one edit pushed
  and pulled 0.29 s. Sync bookkeeping is written in transactions (one wait
  for the disk per batch); uploads and downloads run 8 at a time.
- **Installer:** NSIS, per user. See docs/RELEASE.md for building, code
  signing and auto-update.

## 12. M0 spike findings (2026-10-24)

1. **TipTap ↔ Markdown round-trip** — works, with one fix.
   (`app/src/lib/editor/markdown.test.ts`)
   - Headings, marks, links, nested bullet/ordered/task lists, code blocks,
     blockquotes, images, rules, hard breaks and Unicode round-trip exactly.
   - Tables are re-padded into aligned columns on the first save, then stay
     stable. Side effect: editing one cell can rewrite the whole table's
     lines, so concurrent edits to the same table are more likely to produce
     a conflict copy. Acceptable.
   - Underline is stored as `++text++` (not standard Markdown, but it
     round-trips inside Memoir; export may need to map it).
   - **Bug found and fixed:** TipTap does not escape block syntax at the start
     of a line, so a typed paragraph such as `# idea`, `1. thing`, `- x`,
     `---`, `===` or text with leading spaces reloaded as a different block.
     `SafeParagraph` (`app/src/lib/editor/safeParagraph.ts`) escapes these;
     the test suite covers them, including after hard breaks.
   - Rule for the future: every editor feature must pass the round-trip test
     before it ships.
2. **SQLCipher + FTS5 on Windows** — works.
   (`crates/memoir-core/tests/sqlcipher_fts5.rs`)
   - `rusqlite` 0.41 with `bundled-sqlcipher-vendored-openssl`.
   - FTS5 search is diacritic-insensitive (`gesla` finds `gęślą`) and
     supports prefixes.
   - The file on disk is ciphertext, a wrong key is rejected and the right
     key reopens.
   - Costs: the vendored OpenSSL needs Strawberry Perl (preinstalled on
     GitHub runners) and adds about 5 minutes to a cold build; incremental
     builds are unaffected.
   - On a wrong key SQLCipher logs errors to stderr. Route or silence them
     in M1.
3. **Google OAuth (PKCE, loopback) + Drive API** — works.
   (`crates/memoir-gdrive/examples/oauth_spike.rs`, run 2026-10-24 against a
   personal Google account in "Testing" status)
   - System-browser consent → `127.0.0.1` loopback redirect → code exchange
     with PKCE worked; scope granted is exactly `drive.file`.
   - Refresh token issued (`access_type=offline`) and minted a new access
     token.
   - Created a folder and a file, listed the folder, downloaded the file
     byte-for-byte. Drive reports `version`, `headRevisionId`, `md5Checksum`.
   - **`If-Match` is ignored by Drive v3**: an update with a bogus
     precondition succeeded (HTTP 200). Overwrites are unconditional, which
     confirms the append-only revision design of §6 is required.
   - `reqwest` 0.13 with rustls/aws-lc builds with no NASM/CMake.
4. **Close interception** — works. (`app/src-tauri/src/shutdown.rs`)
   - On WM_CLOSE the window hid at once, a simulated 1.5 s flush ran, and
     the process exited 1.75 s after the close.
   - The 15 s timeout path is in place.
   - Windows logoff/shutdown (`WM_QUERYENDSESSION` + shutdown-block reason)
     is not covered yet; it moves to M6 as planned.

## 13. macOS (M9)

Same code; the differences are in the Tauri shell.

- **Data** in `~/Library/Application Support/com.memoir.notebook`, logs in
  `~/Library/Logs/com.memoir.notebook`, keys in the login Keychain (service
  `com.memoir.notebook`, same accounts as on Windows).
- **Closing and quitting** (`shutdown.rs`, `menu.rs`): as Mac apps do,
  closing the window (red button, ⌘W) only hides it — the editor is saved
  and changes are pushed in the background — and clicking the Dock icon
  shows it again. Quit (⌘Q, or Memoir → Quit Memoir) runs the full closing
  sequence. Quit is Memoir's own menu item because the standard one ends the
  process at once (tao has no `applicationShouldTerminate`). Quitting from
  the Dock, logging out and shutting down still end it at once (tao reports
  them only as `RunEvent::Exit`, which can't be delayed): the last edits
  not yet autosaved (≤ 300 ms of typing) are lost, and saved changes not
  pushed yet go up on next launch (they are safe in the local outbox).
  While quitting, neither the Dock icon nor a menu command shows the window
  again; a close still saving doesn't hide a window reopened meanwhile.
  Requests to save the editor are numbered, so a late answer to one that
  timed out never counts for the next.
- **Menu bar** (`menu.rs`): Memoir (About, Settings… ⌘,, Services, Hide,
  Quit), File (New Note ⌘N, New Folder ⇧⌘N, Search… ⌘K, Close Window),
  the standard Edit menu (which is what makes ⌘C/⌘V/⌘Z work in the
  webview) and Window. Items for the UI arrive as the `menu-command` event.
- **Shortcut labels** in the UI come from `app/src/lib/platform.ts`
  (`keys('Ctrl+Shift+N')` → `⇧⌘N`); the shortcuts themselves already used
  `Mod`/`metaKey`. ⌘⌫ moves the selected tree item to the trash.
- **Bundle:** `.app` and `.dmg` (`tauri.macos.conf.json`), macOS 11+,
  universal (Apple silicon and Intel) in the Release workflow. Signed ad hoc
  until there is a Developer ID; see docs/RELEASE.md.

## 14. iOS (M10, in progress)

Tauri's iOS build of the same app; the Xcode project is generated into
`app/src-tauri/gen/apple` (`tauri ios init`) and checked in.

- **Keychain:** `keyring`'s ready-made setup stops short of iOS (and its
  macOS Keychain store doesn't build there), so on iOS `keystore.rs` uses
  `keyring-core` with the iOS Keychain store
  (`apple-native-keyring-store`, "protected data", the app's own access
  group) directly. Same service and accounts as on the desktop.
- **Data** in the app's sandbox (`Library/Application Support/com.memoir.notebook`).
- **Not on iOS:** the updater (the App Store or TestFlight updates the app),
  the menu bar and window close/quit handling, and Export to Markdown and
  Import from Evernote for now (no folder picker, and `.enex` has no file
  type the iOS picker knows; both will go through the Files app).
- **Phone layout** (`app/src/lib/platform.ts`: `phone` = window ≤ 700 px,
  `touch` = coarse pointer): one pane at a time — the list, or the note /
  trash / settings with a "‹ Notes" back button (`app.listShown`, both panes
  stay mounted so the editor keeps its state); 44 px tree rows with a ⋯
  button for the row menu (no right-click on touch); a one-row scrolling
  toolbar; no keyboard hints; safe-area padding (`viewport-fit=cover`);
  16 px text on touch screens, which also stops iOS zooming into focused
  fields. A narrow desktop window gets the same one-pane layout. With no
  drag & drop on touch, the row menu's "Move to…" picks a folder
  (`sidebar/MoveDialog.svelte`, `moveTargets` in `tree.ts`); it moves to the
  end of that folder, and works on the desktop too.
- **Leaving the app** (`visibilitychange`): the editor is saved and a sync
  starts; coming back pulls. iOS may suspend the app a few seconds later,
  and anything not pushed by then goes up on the next sync. Timers stop
  while the app is suspended, so coming back checks auto-lock first; locking
  returns a phone to the list.
- **Google sign-in:** the desktop's loopback redirect isn't possible on
  iOS. The iOS app uses its own OAuth client (type iOS, no secret,
  `IOS_CLIENT_ID` in `cloud.rs`); Google redirects to the reversed client ID
  as a URL scheme (`Info.ios.plist`), iOS opens Memoir with that URL
  (`RunEvent::Opened`), and `oauth::sign_in_with_app_redirect` checks the
  `state` and exchanges the code (PKCE, no client secret). Same project as
  the desktop client, so both see the same Drive files. The refresh token is
  per device, in the iOS Keychain.
- **Still to do:** a background task so a push can finish after leaving the app,
  share sheet for attachments and export.

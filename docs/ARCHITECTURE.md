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

Each revision's (encrypted) header lists its **parent revision id(s)** —
a tiny DAG per object, like git.

- **Push**: for each outbox entry → encrypt → upload a new revision whose
  parent = last known revision → record it as base.
- **Pull**: `changes.list` with a stored page token (incremental, cheap) →
  download new revisions → for each object:
  - remote revision descends from our base, no local edits → fast-forward
  - local edits and remote changes → **merge**, upload merged revision with
    both parents
- **GC**: a revision superseded by a descendant is deleted after a grace
  period (e.g. 48 h), so a device mid-merge never loses its inputs. Any device
  may GC.

### Merge rules
- Scalar fields (title, parent, sort_key, tags, trashed): last-writer-wins per
  field (timestamp + device id tie-break).
- Body: 3-way text merge (`diffy`) against the stored base. If it can't merge
  cleanly: keep local, save remote as a sibling note
  "‹title› (conflict ‹device› ‹date›)" in the same folder, flag it in the UI.
  **Never silently drop text.**
- Tree anomalies, resolved deterministically on every device:
  - move cycle (A→B on one device, B→A on another) → loser moves to root
  - note's parent folder deleted elsewhere → folder is restored (deletion of a
    non-empty folder never wins against a concurrent addition)

### When sync runs
- After edits (debounce ~3 s), on startup, on window focus, every 60 s while
  open, and on demand.
- Network failures/rate limits: exponential backoff with jitter; resumable
  uploads for large blobs.
- **On close**: window close is intercepted (`CloseRequested` →
  `prevent_close`), window hides, outbox is flushed (timeout ~15 s), then the
  app exits. If offline or timed out, changes stay durable in the outbox and
  are pushed on next launch; the user gets a "N changes not yet synced" notice.
- Windows shutdown/logoff: register a shutdown-block reason while the outbox is
  non-empty, flush, release. Worst case, local data is still safe.
- A **sync status indicator** is always visible: synced / pending N / offline / error.

### Testing the sync engine
`StorageProvider` trait with an in-memory fake that can inject latency,
failures, partial uploads and reordering. Property-based tests (`proptest`)
simulate several devices doing random edits, moves, deletes, crashes and
network loss, then assert: **all devices converge, and no edited text is lost.**

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

## 8. Data ownership & recovery

- **Export**: in-app "Export to Markdown" — decrypted folder tree of `.md` files
  plus attachments.
- **`memoir-cli`**: standalone tool that, given a downloaded copy of the
  `Memoir/` Drive folder and the passphrase or recovery key, decrypts
  everything to Markdown. Guarantees access to your notes even if the app stops
  working. The format is documented in `docs/FORMAT.md` (to be written in M1).

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

## 11. M0 spike findings (2026-10-24)

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

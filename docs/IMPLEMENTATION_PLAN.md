# Memoir — Implementation Plan

Target of this plan: **v1 on Windows**. macOS and iOS follow (M9, M10).
See [ARCHITECTURE.md](ARCHITECTURE.md) for the design.

Each milestone ends with something runnable and a clear "done when".
Milestones are ordered so the riskiest unknowns are resolved first and the app
is usable locally before sync exists.

---

## Status

| Milestone | State |
|---|---|
| M0 | ✅ done |
| M1 | ✅ done |
| M2 | ✅ done (`revisions` / `remote_state` tables moved to M6, where they're used) |
| M3 | ✅ done (plus feedback: note dates, Later list, threads) |
| M4 | ✅ done — attachment cleanup (unreferenced files) moved to M6 |
| M5 | ✅ done — checked against a real Drive (change feed reports deletions ~0.8 s late) |
| M6 | ✅ done — simulation (thousands of runs) and two instances on a real Drive, including a conflict; Windows shutdown-block reason and a sync log moved to M8 |
| M7 | ✅ done — export, `memoir decrypt` (tested end to end with a recovery key), history view |
| M8 | in progress — done: logging, recovery paths, shutdown handling, performance, installer, auto-updates via GitHub Releases, website and Google sign-in published; left: testing and a week of daily use (code signing optional, see docs/RELEASE.md) |
| M10 | in progress — done: builds and runs in the iOS Simulator (`tauri ios dev`); creating a notebook, the iOS Keychain (key remembered across launches) and notes work; iOS compile check in CI; phone layout (one pane at a time with a back button, touch-sized rows and toolbar, ⋯ menu on rows, no keyboard hints, safe areas); save and sync when the app goes to the background; Google sign-in with an iOS OAuth client and the app's URL scheme (opened the real notebook from Drive in the Simulator). Left: a background task so pushes can finish after leaving the app, moving notes without drag & drop, export and attachments through the share sheet / Files, square opaque app icon, running on a real iPhone (Apple developer team), TestFlight |
| M9 | in progress — done: builds and runs on macOS, Keychain, menu bar, close/quit/Dock conventions, ⌘ shortcut labels, `.app`/`.dmg` with auto-updates (universal build in the Release workflow), macOS in CI; left: Developer ID signing and notarization (needs an Apple Developer account, see docs/RELEASE.md), a week of daily use on a Mac |

## M0 — Foundations & spikes
Set up the project and kill the big unknowns before building on them.

- Cargo workspace: `memoir-core`, `memoir-gdrive`, `memoir-cli`, `app/src-tauri`
- Tauri 2 + Svelte 5 + TypeScript + Vite app shell; `tauri-specta` bindings
- Tooling: `rustfmt`, `clippy -D warnings`, `cargo test`, ESLint/Prettier,
  `svelte-check`; one `just`/npm script to run everything
- git repo + GitHub Actions CI on `windows-latest`
- **Spikes** (throwaway code, findings written into ARCHITECTURE.md):
  1. SQLCipher + FTS5 build and query on Windows
  2. TipTap ⇄ Markdown round-trip of headings, lists, checklists, tables,
     code, images, links
  3. Google OAuth PKCE loopback from Tauri, list a Drive folder
  4. Intercept window close, run async work, then exit

**Done when:** `npm run tauri dev` opens an empty window on Windows, CI is
green, and all four spikes have a documented "works / workaround".

## M1 — Crypto & vault
- Envelope format (encrypt/decrypt, AAD binding, zstd), chunked blob streams
- Key hierarchy: Argon2id KEK, Master Key, HKDF subkeys, recovery key
  generation (human-friendly encoding)
- `vault.json` create / unlock / change passphrase
- Keychain caching of `local_db_key`
- `docs/FORMAT.md` — byte-level spec of envelope, vault and object encoding
- Tests: round-trips, tamper detection (flipped bit, swapped file, wrong AAD),
  known-answer vectors, zeroization of secrets

**Done when:** core library can create a vault, lock/unlock it, and
encrypt/decrypt objects and blobs, with full test coverage of failure cases.

## M2 — Local store
- SQLCipher schema + migrations (`objects`, `notes_fts`, `revisions`,
  `outbox`, `remote_state`, `history`)
- Domain API in `memoir-core`: create/rename/move/reorder/trash/restore/purge
  for folders and notes; fractional `sort_key`; get tree; get/save note;
  search
- Every mutation writes object + FTS + outbox in one transaction
- Benchmark harness: generate 10k notes, measure open / save / search

**Done when:** all domain operations are covered by tests, and benchmarks meet
the targets (open < 50 ms, search < 30 ms on 10k notes).

## M3 — UI shell: tree + editor
- First-run flow: create vault (passphrase + show/confirm recovery key);
  unlock screen
- Two-pane layout with resizable splitter
- **Left: tree** — virtualized, expand/collapse, folder and note icons,
  inline rename, context menu (new note / new folder / rename / trash),
  drag & drop move and reorder, keyboard navigation, remembers expanded state
- **Right: editor** — TipTap with the feature set validated in M0, title
  field, autosave (debounced 300 ms), no save button
- Search box (Ctrl+K / Ctrl+P): instant results, Enter opens note
- Trash view with restore / delete forever
- Keyboard shortcuts for common actions; light/dark theme following Windows

**Done when:** the app is fully usable as a local, encrypted notebook on
Windows — create a folder tree, write notes, search, move things around,
restart and find everything intact.

## M4 — Attachments
- Paste / drag & drop images and files into the editor
- Encrypted local blob store, HMAC-based blob IDs, dedup
- Images render inline; other files as attachment chips (open / save as)
- Size limit for v1 (e.g. 100 MB per file)

**Done when:** images and files can be added, survive restart, and are stored
encrypted on disk.

## M5 — Google Drive provider
- `StorageProvider` trait in `memoir-core` (list, changes since token,
  download, create, delete, resumable upload)
- `memoir-gdrive` implementation: OAuth PKCE, token refresh, refresh token in
  Windows Credential Manager, `Memoir/` folder bootstrap, backoff on 429/5xx
- In-memory fake provider with fault injection (for M6 tests)
- Settings UI: connect / disconnect Google account
- Google Cloud project set up with `drive.file` scope (stays in "Testing"
  until M8)

**Done when:** the app can sign in, create the `Memoir/` folder, upload and
download encrypted files, and survive token expiry and flaky network.

## M6 — Sync engine (the critical milestone)
- Revision DAG: push new revisions, pull via `changes.list`, fast-forward,
  merge (field-level LWW + 3-way body merge), conflict copies
- Tree anomaly resolution (cycles, orphans), deterministic on all devices
- Blob sync (upload before the note revision that references it)
- Revision GC with grace period
- Attachment cleanup: delete local and remote attachments no longer referenced by any note, reply or history version (after a grace period)
- Sync triggers: edit debounce, startup, focus, 60 s timer, manual
- **Close-time flush** and Windows shutdown-block reason
- Second-device setup: sign in → download vault → passphrase → full pull
- Status indicator + "pending changes" notice; sync log for debugging
- **Property-based multi-device simulation tests** against the fake provider:
  random edits/moves/deletes/crashes/network failures → convergence + no
  lost text. Run thousands of cases in CI.

**Done when:** two Windows machines (or two profiles on one) editing the same
notebook — including offline edits and simultaneous edits to the same note —
always converge with no lost text; closing the app pushes pending changes.

## M7 — Data ownership & recovery
- In-app "Export to Markdown" (folders → directories, notes → `.md`,
  attachments alongside)
- `memoir-cli decrypt <drive-folder> --out <dir>` using passphrase or
  recovery key
- Local note history view (restore an earlier version)

**Done when:** a copy of the Drive folder + recovery key is enough to get all
notes back as Markdown, without the app.

## M8 — Hardening & Windows release
- Performance pass against targets; startup profiling
- Error handling & recovery paths (corrupt revision file, missing blob,
  revoked Google access, keychain wiped)
- Windows installer (NSIS/MSI via Tauri bundler), code signing
- Auto-update via Tauri updater (signed update manifests)
- Crash-safe logging (no note content in logs), including a sync log for
  debugging (moved from M6)
- Windows shutdown-block reason while changes are unsynced (moved from M6;
  until then, changes not pushed at shutdown go up on next launch)
- Google OAuth app out of "Testing" (which signs you out every 7 days):
  homepage + privacy policy on GitHub Pages, verified domain, publish — see
  [spikes/google-oauth.md](spikes/google-oauth.md) step 4

**Done when:** installable, self-updating Windows build used daily for a week
without data issues.

---

## Later
- **M9 — macOS**: build, Keychain, menu bar conventions, notarization
  (in progress, see Status)
- **M10 — iOS**: Tauri mobile build, touch layout (tree becomes a drill-down
  navigation), OAuth via custom URL scheme, background sync limits (flush on
  app backgrounding instead of close)
- **Editor iterations**: refine the feature set and feel after real use
- iCloud provider (only if wanted once Apple platforms are in)

## Suggested order of work within the team of "you + Claude"
M0 → M1 → M2 → M3 gives a usable local app quickly (and the first feedback on
the editor). M4 can slip after M6 if sync feels more urgent. M6 deserves the
most time and testing — it is where reliability is won or lost.

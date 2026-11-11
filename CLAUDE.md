# Memoir

Personal, end-to-end encrypted, offline-first notebook (Rust core + Tauri 2,
Svelte 5 UI). Design and decisions: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md);
storage format: [docs/FORMAT.md](docs/FORMAT.md); releases: [docs/RELEASE.md](docs/RELEASE.md).

## Keep this file current

When an important decision or a working preference comes up — from the
user, or learned the hard way — add it here (or, for design decisions, to
docs/ARCHITECTURE.md) in the same change. Keep entries short and say why.

## Working preferences

- **Platforms:** "all platforms" means Windows, macOS and iOS. Features go
  into the shared code, so they work on all three; check the phone layout too.
- **Branches and PRs:** do feature work on its own branch and open a PR to
  `main`; don't commit to `main` directly.
- **Docs:** record a feature's design in docs/ARCHITECTURE.md as part of the
  same PR.
- **Editor features** must pass the Markdown round-trip test
  (`app/src/lib/editor/markdown.test.ts`) before they ship.

## Environment

- Node (`~/.local/node/bin`) and Rust (`~/.cargo/bin`) are installed in the
  user folder and put on the PATH by `~/.zprofile`. A shell that doesn't
  read it needs `export PATH="$HOME/.cargo/bin:$HOME/.local/node/bin:$PATH"`.
- `build-app.sh` (Mac release build, git-ignored) holds the Google client ID
  and secret: never commit it or quote its contents.
- `npm run dev` (in `app/`) runs the UI in a browser against an in-memory
  mock with sample notes (`app/src/lib/api/mock.ts`; `?mock=first_run` /
  `?mock=locked` for the other screens).
- Before a PR: `npm run verify` in `app/` (type check, lint, UI tests,
  rustfmt, clippy, Rust tests).
- iOS builds (`tauri ios dev` / `build`) need `LANG=en_US.UTF-8`, or
  CocoaPods fails. `Info.ios.plist` (URL schemes and the like) is merged
  only when a dev run starts: restart `tauri ios dev` after changing it.
- Xcode and Tauri rewrite `app/src-tauri/gen/apple` when they open or
  regenerate the project; changes there that are only formatting can be
  discarded.
- The Simulator holds a synced copy of the user's real notebook, and its
  passphrase is theirs to type: try UI changes in the browser mock at phone
  size instead.
- iPhone builds go out through TestFlight: `app/scripts/ios-testflight.sh`
  builds, signs and uploads (`--no-upload` only exports the `.ipa`). Ask
  before uploading. Setup and pitfalls: docs/RELEASE.md, "iPhone".

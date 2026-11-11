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

- `node` and `npm` are in `~/.local/node/bin`, which isn't on the shell's
  PATH: `export PATH="$HOME/.local/node/bin:$PATH"`.
- `npm run dev` (in `app/`) runs the UI in a browser against an in-memory
  mock with sample notes (`app/src/lib/api/mock.ts`; `?mock=first_run` /
  `?mock=locked` for the other screens).
- Before a PR: `npm run verify` in `app/` (type check, lint, UI tests,
  rustfmt, clippy, Rust tests).
- iPhone builds go out through TestFlight: `app/scripts/ios-testflight.sh`
  builds, signs and uploads (`--no-upload` only exports the `.ipa`). Ask
  before uploading. Setup and pitfalls: docs/RELEASE.md, "iPhone".

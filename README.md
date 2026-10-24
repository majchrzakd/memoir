# Memoir

A personal, end-to-end encrypted, offline-first notebook that syncs through
your own Google Drive.

- Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Roadmap: [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md)

## Development (Windows)

Prerequisites: Rust (stable, via rustup), Node.js LTS, Visual Studio C++ build
tools, and [Strawberry Perl](https://strawberryperl.com/) (builds the vendored
OpenSSL used by SQLCipher).

```powershell
cd app
npm install
npm run tauri dev      # run the app
npm run verify         # everything CI runs: types, lint, tests, fmt, clippy
```

Layout:

| Path | What |
|---|---|
| `crates/memoir-core` | domain logic: vault, crypto, local store, sync |
| `crates/memoir-gdrive` | Google Drive provider |
| `crates/memoir-cli` | offline recovery/export tool |
| `app/src-tauri` | Tauri shell |
| `app/src` | Svelte UI |

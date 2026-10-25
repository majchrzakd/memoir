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

To work on the UI alone, `npm run dev` and open <http://localhost:5173> in a
browser: it runs against an in-memory mock backend with sample notes
(`?mock=first_run` or `?mock=locked` for the other screens; the locked mock's
passphrase is `correct horse`).

The app keeps its data in `%LOCALAPPDATA%\com.memoir.notebook` (`vault.json`,
`memoir.db`, `blobs\` with encrypted attachments); the database key is remembered in Windows Credential Manager
under `com.memoir.notebook`. Development builds can use another folder by setting
`MEMOIR_DATA_DIR` (handy for testing without touching your notebook).

Layout:

| Path | What |
|---|---|
| `crates/memoir-core` | domain logic: vault, crypto, local store, sync |
| `crates/memoir-gdrive` | Google Drive provider |
| `crates/memoir-cli` | offline recovery/export tool |
| `app/src-tauri` | Tauri shell |
| `app/src` | Svelte UI |

# Releasing Memoir for Windows

## Build the installer

1. Bump the version in `app/src-tauri/tauri.conf.json` (and `Cargo.toml`'s
   `workspace.package.version`).
2. In the PowerShell window you build from, set the Google client (it is
   built into the app; see [spikes/google-oauth.md](spikes/google-oauth.md)):
   ```powershell
   $env:MEMOIR_GOOGLE_CLIENT_ID = "<client id>"
   $env:MEMOIR_GOOGLE_CLIENT_SECRET = "<client secret>"
   ```
   A build without them works, but can't connect to Google Drive.
3. Build:
   ```powershell
   cd app
   npm.cmd run tauri build
   ```
   Steps 2 and 3 in one go: `build-installer.bat` in the project folder
   (local only, ignored by git) holds the keys and runs the build.

   The installer is `target\release\bundle\nsis\Memoir_<version>_x64-setup.exe`.
   It installs for the current user (no administrator rights needed) and
   fetches the WebView2 runtime if Windows lacks it. Uninstalling leaves the
   notebook in `%LOCALAPPDATA%\com.memoir.notebook`.

The icon is drawn in `app/src-tauri/icons/memoir.svg`; after changing it,
regenerate the sizes with `npm.cmd run tauri icon src-tauri/icons/memoir.svg`
(and delete the `android`/`ios` folders it also creates, until those
platforms are built).

## Decisions and accounts still needed

### Code signing
Without a signature, Windows SmartScreen warns ("Windows protected your
PC") when the installer runs — you click *More info → Run anyway*. That's
fine for personal use. To avoid it:
- **Azure Trusted Signing** (about $10/month; needs an Azure account and an
  identity check), or
- a code-signing certificate from a CA (OV certificates still show the
  warning until the file builds up reputation; EV certificates don't).

Signing is then configured in `bundle.windows` (`certificateThumbprint` or
`signCommand`).

## Releases and auto-update (GitHub)

Installed copies check
`https://github.com/majchrzakd/memoir/releases/latest/download/latest.json`
about 30 s after starting and every 12 hours. They show "Memoir X is
available"; **Restart to update** saves, pushes pending changes, installs
and restarts. Settings → About has "Check for updates". Updates must be
signed with the project's update key; the public half is
`plugins.updater.pubkey` in `tauri.conf.json`.

### One-time setup
1. **Update signing key** (separate from code signing):
   ```powershell
   cd app
   npm.cmd run tauri signer generate -- -w "$env:USERPROFILE\.tauri\memoir.key"
   ```
   Keep `memoir.key` and its password safe and backed up (a password
   manager). If either is lost, installed copies can never update again.
   `memoir.key.pub` (public) goes into `tauri.conf.json`.
2. **Repository secrets** (GitHub → the repo → Settings → Secrets and
   variables → Actions → New repository secret):
   - `TAURI_SIGNING_PRIVATE_KEY`: the whole content of `memoir.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password
   - `MEMOIR_GOOGLE_CLIENT_ID` and `MEMOIR_GOOGLE_CLIENT_SECRET`: from
     `google_keys.txt`

### Each release
1. Set the new version in `app/src-tauri/tauri.conf.json` (`version`) and
   in `Cargo.toml` (`workspace.package.version`), commit and push.
2. GitHub → Actions → **Release** → Run workflow. It builds and signs on
   Windows and creates a **draft** release `v<version>` with the installer,
   its `.sig` and `latest.json`.
3. Check the draft (add notes: they're shown in the update notice) and
   **Publish**. Installed copies pick it up on their next check.

Builds made with `build-installer.bat` skip the update signature
(`tauri.local-build.conf.json`), so they need no signing key. They still
update themselves from GitHub releases like any installed copy.

### Google sign-in out of "Testing"
See [spikes/google-oauth.md](spikes/google-oauth.md) step 4. The homepage
and privacy policy are drafted in `docs/site/`. Fill in `CONTACT_EMAIL` in
both pages, publish them (e.g. GitHub Pages), verify the domain in Google
Search Console, enter the URLs on the Branding page, and publish the app.
With only the `drive.file` scope, Google doesn't need to review it.

## Before each release
- `npm.cmd run verify` (types, lint, tests, formatting, clippy)
- A longer sync simulation:
  `$env:MEMOIR_SIM_SEEDS=1000; $env:MEMOIR_SIM_STEPS=150; cargo test -p memoir-core --release --test sync_sim -- --nocapture`
- Sync timings for a big notebook:
  `cargo test -p memoir-core --release --test sync_bench -- --ignored --nocapture`

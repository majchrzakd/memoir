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

### Auto-update
Tauri's updater checks a small JSON file for a newer, signed installer. It
needs:
1. **A place for the files**, e.g. GitHub Releases. The project has no
   remote repository yet.
2. **An update signing key** (separate from code signing):
   `npm.cmd run tauri signer generate -- -w %USERPROFILE%\.tauri\memoir.key`.
   Keep the private key and its password safe and backed up. If it's lost,
   installed copies can never update again. The public key goes into
   `tauri.conf.json`.
3. Then: add `tauri-plugin-updater`, the endpoint URL, a "Check for updates"
   step at startup (asking before installing), and set
   `TAURI_SIGNING_PRIVATE_KEY` when building, so the build also produces the
   `.sig` file and `latest.json` to upload with the installer.

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

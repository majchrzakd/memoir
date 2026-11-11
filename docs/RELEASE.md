# Releasing Memoir (Windows, macOS and iPhone)

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

### On a Mac

Same steps in Terminal (`export MEMOIR_GOOGLE_CLIENT_ID=…` and
`export MEMOIR_GOOGLE_CLIENT_SECRET=…`), then:
```sh
cd app
npm run tauri build -- --config src-tauri/tauri.local-build.conf.json
```
Or in one go: `build-app.sh` in the project folder (local only, ignored by
git) holds the keys, builds and installs into `/Applications`.
This builds for the Mac it runs on: `target/release/bundle/macos/Memoir.app`
and `target/release/bundle/dmg/Memoir_<version>_aarch64.dmg`. (A universal
build for Apple silicon and Intel needs
`rustup target add aarch64-apple-darwin x86_64-apple-darwin` and
`--target universal-apple-darwin`; the Release workflow does that.)
Uninstalling (dragging Memoir to the Bin) leaves the notebook in
`~/Library/Application Support/com.memoir.notebook`.

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

### macOS signing and notarization
Without the Apple secrets below, the Mac app is signed *ad hoc*. It runs,
but:
- the first time, macOS refuses to open an app downloaded from the internet
  that isn't notarized. Open it once with right-click → **Open** (or System
  Settings → Privacy & Security → **Open Anyway**); updates after that are
  fine;
- after an update the Keychain may ask again whether Memoir can use its saved
  key (choose **Always Allow**), because an ad-hoc signature changes with
  every build.

To remove both, the Release workflow signs the Mac app with a *Developer ID
Application* certificate and has Apple notarize it. The Apple Developer
Program membership (team `SXRB4AG285`, the same one TestFlight uses) covers
it. Builds made on your Mac (`build-app.sh`) stay ad hoc.

One-time setup:
1. **Certificate** (only the team's Account Holder can create one): Xcode →
   Settings → Accounts → the team → **Manage Certificates…** → **+** →
   *Developer ID Application*. It lands in the login keychain; check with
   `security find-identity -v -p codesigning`.
2. **Export it:** Keychain Access → login → My Certificates → right-click
   *Developer ID Application: …* → Export → `.p12`, with a new password.
   Keep the `.p12` and its password in a password manager.
3. **App-specific password** for notarization: account.apple.com → Sign-In
   and Security → App-Specific Passwords.
4. **Repository secrets**, from the project folder (each `gh secret set NAME`
   asks for the value):
   ```sh
   base64 -i ~/Desktop/memoir-developer-id.p12 | gh secret set APPLE_CERTIFICATE
   gh secret set APPLE_CERTIFICATE_PASSWORD   # the .p12 export password
   gh secret set APPLE_SIGNING_IDENTITY       # "Developer ID Application: <name> (SXRB4AG285)"
   gh secret set APPLE_ID                     # the Apple Account email
   gh secret set APPLE_PASSWORD               # the app-specific password
   gh secret set APPLE_TEAM_ID --body SXRB4AG285
   ```
   Then delete the `.p12` from the Desktop.

The Release workflow's *Apple signing* step switches on when
`APPLE_CERTIFICATE` is set and fails if any of the others is missing. The
`APPLE_SIGNING_IDENTITY` it passes on replaces the ad-hoc `"-"` in
`tauri.macos.conf.json`. Check the first signed run's macOS log for the
notarization, and `spctl -a -vv /Applications/Memoir.app` on the installed
app ("source=Notarized Developer ID").

The first update from an ad-hoc build to a signed one asks the Keychain
question once more (the signature changes one last time); after that it
stays quiet. A Developer ID certificate is valid for 5 years; renew it and
replace the first three secrets before it runs out.

The Mac App Store was considered and left for later: it requires the App
Sandbox (the notebook would move into the app's container, and export,
import and the loopback Google sign-in need sandbox entitlements) and
doesn't allow the self-updater, so it would be a second build. If it comes,
add macOS to the iPhone app's App Store record (same bundle ID, one listing).

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
1. Push what should go into the release.
2. GitHub → Actions → **Release** → Run workflow, and choose the step:
   **patch** (0.1.0 → 0.1.1, the default), **minor** (→ 0.2.0) or **major**
   (→ 1.0.0), counted from the newest published release. The workflow sets
   that version in `app/src-tauri/tauri.conf.json`, `Cargo.toml` and
   `Cargo.lock` and commits it ("Version 0.1.1"), so pull afterwards. Running
   it again before publishing keeps the version (no new commit) and replaces
   the draft; a version set higher by hand is kept too.

   It then builds and signs on Windows and macOS and creates a **draft**
   release `v<version>` with the Windows installer, the Mac `.dmg` and
   `.app.tar.gz` (what updates download), their `.sig` files and
   `latest.json`. An older draft for the
   same version is replaced, so there's no need to delete it first, but
   don't delete the new draft while the run is going.
3. Check the draft (add notes: they're shown in the update notice) and
   **Publish**. Installed copies pick it up on their next check.

Builds made with `build-installer.bat` skip the update signature
(`tauri.local-build.conf.json`), so they need no signing key. They still
update themselves from GitHub releases like any installed copy.

## iPhone (TestFlight)

The iPhone app goes out through TestFlight. Each uploaded build works for
90 days after its upload; a newer build replaces it on the phone (TestFlight
can update it automatically) and keeps the app's data and Keychain.

### One-time setup
1. **Xcode account:** Xcode → Settings → Accounts, sign in with the Apple ID
   of the developer team (`bundle.iOS.developmentTeam` in
   `app/src-tauri/tauri.ios.conf.json`). The team must not be listed as a
   "Personal Team" any more.
2. **Register an iPhone once:** Xcode's automatic signing signs the archive
   for development first, and that needs at least one device on the team,
   even though TestFlight installs need none. Connect the iPhone with a
   cable, unlock it, tap *Trust*, and turn on Developer Mode (Settings →
   Privacy & Security → Developer Mode; the phone restarts). Connecting
   alone doesn't register it; building for it does:
   ```sh
   xcrun devicectl list devices   # the iPhone's identifier (UDID)
   cd app/src-tauri/gen/apple
   xcodebuild -project memoir-app.xcodeproj -scheme memoir-app_iOS \
     -configuration release -destination id=<UDID> \
     -allowProvisioningUpdates -allowProvisioningDeviceRegistration build
   ```
   It registers the phone and creates the signing profile, then stops at
   "Build Rust Code" (that step only runs under `tauri ios build`), which is
   fine.
3. **App record:** [App Store Connect](https://appstoreconnect.apple.com) →
   Apps → **+** → New App: platform iOS, name Memoir, bundle ID
   `com.memoir.notebook` (it's in the list once the first build has
   registered it, or add it under Certificates, Identifiers & Profiles), any
   SKU.
4. **Testers:** the app → TestFlight → Internal Testing → **+**, add yourself.
   Install the TestFlight app on the iPhone and accept the invitation.

### Each build
```sh
app/scripts/ios-testflight.sh
```
It archives a release build, signs it for the App Store and uploads it. The
build number is the build's date and time (`YYYYMMDDHHMM`), so every upload
is newer than the last; the version shown is `version` from
`tauri.conf.json`. `--no-upload` only exports
`app/src-tauri/gen/apple/build/testflight/Memoir.ipa`.

After Apple processes it (usually 5–15 minutes) the build shows up under
TestFlight. The first time, App Store Connect asks about encryption
(*Missing Compliance*): Memoir encrypts notebooks with ChaCha20-Poly1305
(`memoir-core`), so answer that question for the build there; once it's
settled, `ITSAppUsesNonExemptEncryption` in `Info.ios.plist` can skip the
question for later builds.

The iOS icon is drawn in `app/src-tauri/icons/memoir-ios.svg` (full-bleed
and opaque: iOS rounds the corners, and App Store Connect rejects icons with
an alpha channel). After changing it, run `app/scripts/ios-icons.sh`.

### Google sign-in out of "Testing"
See [spikes/google-oauth.md](spikes/google-oauth.md) step 4. The homepage
and privacy policy are in `docs/site/`; the Website workflow publishes them
to https://majchrzakd.github.io/memoir/ (homepage) and
https://majchrzakd.github.io/memoir/privacy.html (privacy policy) — one-time:
repo Settings → Pages → Source: "GitHub Actions". Then verify the domain in
Google Search Console, enter the URLs on the Branding page, and publish the
app. With only the `drive.file` scope, Google doesn't need to review it.

## Before each release
- `npm.cmd run verify` (types, lint, tests, formatting, clippy)
- A longer sync simulation:
  `$env:MEMOIR_SIM_SEEDS=1000; $env:MEMOIR_SIM_STEPS=150; cargo test -p memoir-core --release --test sync_sim -- --nocapture`
- Sync timings for a big notebook:
  `cargo test -p memoir-core --release --test sync_bench -- --ignored --nocapture`

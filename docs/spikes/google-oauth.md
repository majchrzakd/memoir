# Google Cloud setup (needed for Drive sync)

Memoir talks to Google Drive with your own OAuth client. This is a one-time
setup, about 10 minutes, done in the Google Cloud Console with the Google
account whose Drive will hold your notes.

## 1. Project and API
1. Open <https://console.cloud.google.com/> → project picker → **New project**,
   name it `Memoir`.
2. **APIs & Services → Library** → search **Google Drive API** → **Enable**.

## 2. OAuth consent ("Google Auth Platform")
1. **Google Auth Platform → Branding**: app name `Memoir`, your email as
   support and developer contact. Save.
2. **Audience**: user type **External**. Add your own Google account under
   **Test users** for now.
3. **Data access → Add or remove scopes**: add
   `https://www.googleapis.com/auth/drive.file` only. It is a non-sensitive
   scope: the app can only see files it created itself, never the rest of your
   Drive.

## 3. Desktop client
1. **Clients → Create client** → application type **Desktop app**, name
   `Memoir desktop`.
2. Copy the **Client ID** and **Client secret**. (For desktop apps Google
   treats the secret as non-confidential; PKCE protects the flow. Still, keep
   it out of git.)

## 4. Publish — later, before daily use (M8)
**Skip this during development.** "Testing" status works fully for your own
account (listed under **Test users**); the only drawback is that Google
expires refresh tokens after 7 days, so you have to sign in again weekly.

To publish ("In production"), Google requires the **Branding** page to have
an app name, support email, **homepage URL** and **privacy policy URL**, and
the homepage's domain must be an authorized domain you have verified in
Google Search Console. Plan for M8:
1. Host the homepage and privacy policy in `docs/site/` (replace
   `CONTACT_EMAIL` first) on GitHub Pages (`<username>.github.io/memoir`).
2. Verify `<username>.github.io` in Search Console (HTML-file method) and add
   it under **Branding → Authorized domains**.
3. Fill in the URLs and click **Publish app**. With only the `drive.file`
   scope no Google review is required.

## 5. Give Memoir your client
The app reads the client from two environment variables — at build time
(release builds) or at run time (development). In the PowerShell window you
build or run from:

```powershell
$env:MEMOIR_GOOGLE_CLIENT_ID = "<client id>"
$env:MEMOIR_GOOGLE_CLIENT_SECRET = "<client secret>"
npm.cmd --prefix app run tauri dev
```

Then **Settings → Connect Google Drive**. Without the variables, Settings
says the build has no Google sign-in configured.

## 6. Check the Drive connection (optional)
With the same variables set:

```powershell
cargo run -p memoir-gdrive --example drive_check
```

It signs in through the browser, then — in a separate `Memoir check` folder,
never your real `Memoir` one — uploads a small and a 12 MB file, lists,
downloads, follows the change feed and deletes them again, and finally
revokes its access. Delete the empty `Memoir check` folder afterwards.

While the OAuth app is in **Testing**, Google expires the sign-in after 7
days; Memoir then shows "Connect again" in Settings.

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

## 4. Publish (before daily use)
While the app is in **Testing**, Google expires refresh tokens after 7 days,
so you'd be signed out weekly. Under **Audience**, click **Publish app**
("In production"). With only the `drive.file` scope no Google review is
required.

## 5. Run the spike
```powershell
$env:MEMOIR_GOOGLE_CLIENT_ID = "<client id>"
$env:MEMOIR_GOOGLE_CLIENT_SECRET = "<client secret>"
cargo run -p memoir-gdrive --example oauth_spike
```
A browser tab opens for consent; afterwards the terminal prints the results
of the four checks. It creates a `Memoir spike` folder in your Drive, which
you can delete afterwards.

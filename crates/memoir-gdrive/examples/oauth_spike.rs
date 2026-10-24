//! M0 spike 3: Google OAuth (Authorization Code + PKCE, loopback redirect)
//! and the Drive API behaviour the sync design depends on.
//!
//! ```powershell
//! $env:MEMOIR_GOOGLE_CLIENT_ID = "....apps.googleusercontent.com"
//! $env:MEMOIR_GOOGLE_CLIENT_SECRET = "..."
//! cargo run -p memoir-gdrive --example oauth_spike
//! ```
//!
//! Checks, in order:
//! 1. consent in the system browser, redirect to a loopback port, code exchange
//! 2. a refresh token is issued and can mint a new access token
//! 3. with the `drive.file` scope: create a folder and a file, list, download
//! 4. whether Drive v3 honours `If-Match` (compare-and-swap) on updates
//!
//! Everything it creates lives in a Drive folder named "Memoir spike".

use std::error::Error;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const DRIVE: &str = "https://www.googleapis.com/drive/v3";
const DRIVE_UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const SPIKE_FOLDER: &str = "Memoir spike";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    scope: String,
}

#[derive(Deserialize)]
struct FileList {
    files: Vec<DriveFile>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // fields are only printed
struct DriveFile {
    id: String,
    name: String,
    version: Option<String>,
    head_revision_id: Option<String>,
    md5_checksum: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let client_id = std::env::var("MEMOIR_GOOGLE_CLIENT_ID")
        .map_err(|_| "set MEMOIR_GOOGLE_CLIENT_ID (see docs/spikes/google-oauth.md)")?;
    let client_secret = std::env::var("MEMOIR_GOOGLE_CLIENT_SECRET")
        .map_err(|_| "set MEMOIR_GOOGLE_CLIENT_SECRET (see docs/spikes/google-oauth.md)")?;
    let http = reqwest::Client::new();

    // 1. Authorization code + PKCE over a loopback redirect.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect_uri = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let verifier = random_token(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(24);

    let auth_url = Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ],
    )?;
    println!("[1] Opening the browser for Google consent…\n    {auth_url}");
    open::that(auth_url.as_str())?;
    let code = wait_for_redirect(&listener, &state).await?;

    let tokens: TokenResponse = http
        .post(TOKEN_URL)
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "    ok: access token valid {}s, scope = {}",
        tokens.expires_in, tokens.scope
    );

    // 2. Refresh token.
    let refresh_token = tokens
        .refresh_token
        .ok_or("no refresh token issued (access_type=offline ignored?)")?;
    let refreshed: TokenResponse = http
        .post(TOKEN_URL)
        .form(&[
            ("refresh_token", refresh_token.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!("[2] ok: refresh token minted a new access token");
    let token = refreshed.access_token;

    // 3. drive.file: folder, file, list, download.
    let folder_id = find_or_create_folder(&http, &token).await?;
    println!("[3] folder '{SPIKE_FOLDER}' id = {folder_id}");

    let content = b"MMR1 opaque ciphertext would go here";
    let file = upload_new(&http, &token, &folder_id, "probe.mmr", content).await?;
    println!("    created {file:?}");

    let listed: FileList = http
        .get(format!("{DRIVE}/files"))
        .bearer_auth(&token)
        .query(&[
            ("q", format!("'{folder_id}' in parents and trashed = false")),
            (
                "fields",
                "files(id,name,version,headRevisionId,md5Checksum)".into(),
            ),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!("    folder lists {} file(s)", listed.files.len());

    let downloaded = http
        .get(format!("{DRIVE}/files/{}", file.id))
        .bearer_auth(&token)
        .query(&[("alt", "media")])
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    assert_eq!(&downloaded[..], content);
    println!("    download matches upload");

    // 4. Does Drive v3 reject an update whose If-Match doesn't match?
    let response = http
        .patch(format!("{DRIVE_UPLOAD}/files/{}", file.id))
        .bearer_auth(&token)
        .query(&[("uploadType", "media")])
        .header("If-Match", "\"definitely-not-the-current-etag\"")
        .body(&b"overwritten despite stale precondition"[..])
        .send()
        .await?;
    match response.status().as_u16() {
        412 => println!("[4] If-Match IS honoured (412) — compare-and-swap is available"),
        s if (200..300).contains(&s) => {
            println!("[4] If-Match is IGNORED (status {s}) — overwrites are unconditional")
        }
        s => println!("[4] inconclusive: status {s}: {}", response.text().await?),
    }

    println!("\nDone. You can delete the '{SPIKE_FOLDER}' folder from Drive.");
    Ok(())
}

/// Serves exactly one OAuth redirect on the loopback listener.
async fn wait_for_redirect(listener: &TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);
        let Some(target) = request
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("GET "))
            .and_then(|l| l.split(' ').next())
        else {
            continue;
        };
        let url = Url::parse(&format!("http://localhost{target}"))?;
        let param = |k: &str| {
            url.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        };

        // Browsers also ask for /favicon.ico etc.
        if param("code").is_none() && param("error").is_none() {
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await;
            continue;
        }

        let body = "<html><body style='font-family:system-ui'>Memoir: sign-in complete. You can close this tab.</body></html>";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;

        if let Some(err) = param("error") {
            return Err(format!("authorization failed: {err}").into());
        }
        if param("state").as_deref() != Some(expected_state) {
            return Err("state mismatch — possible CSRF, aborting".into());
        }
        return Ok(param("code").expect("checked above"));
    }
}

async fn find_or_create_folder(http: &reqwest::Client, token: &str) -> Result<String> {
    let found: FileList = http
        .get(format!("{DRIVE}/files"))
        .bearer_auth(token)
        .query(&[
            (
                "q",
                format!(
                    "name = '{SPIKE_FOLDER}' and mimeType = '{FOLDER_MIME}' and trashed = false"
                ),
            ),
            ("fields", "files(id,name)".into()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if let Some(f) = found.files.into_iter().next() {
        return Ok(f.id);
    }

    let created: DriveFile = http
        .post(format!("{DRIVE}/files"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "name": SPIKE_FOLDER, "mimeType": FOLDER_MIME }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(created.id)
}

async fn upload_new(
    http: &reqwest::Client,
    token: &str,
    folder_id: &str,
    name: &str,
    content: &[u8],
) -> Result<DriveFile> {
    let boundary = format!("memoir-{}", random_token(12));
    let metadata = serde_json::json!({ "name": name, "parents": [folder_id] });
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n\
             --{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    Ok(http
        .post(format!("{DRIVE_UPLOAD}/files"))
        .bearer_auth(token)
        .query(&[
            ("uploadType", "multipart"),
            ("fields", "id,name,version,headRevisionId,md5Checksum"),
        ])
        .header(
            "Content-Type",
            format!("multipart/related; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random number generator");
    URL_SAFE_NO_PAD.encode(buf)
}

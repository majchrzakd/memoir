//! Google Drive as a [`StorageProvider`].
//!
//! Layout in the user's Drive (only files this app created are visible to it,
//! thanks to the `drive.file` scope):
//!
//! ```text
//! Memoir/          Area::Root
//!   objects/       Area::Objects
//!   blobs/         Area::Blobs
//! ```

use std::sync::RwLock;
use std::time::Duration;

use memoir_core::remote::{Area, Changes, ProviderError, RemoteFile, Result, StorageProvider};
use reqwest::header::{CONTENT_RANGE, CONTENT_TYPE, LOCATION, RANGE, RETRY_AFTER};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::oauth::{self, AccessToken, OAuthConfig, OAuthError};

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const FILE_FIELDS: &str = "id,name,size,modifiedTime";
const ROOT_FOLDER: &str = "Memoir";

#[derive(Debug, Clone)]
pub struct DriveConfig {
    pub oauth: OAuthConfig,
    pub api_url: String,
    pub upload_url: String,
    pub retry: RetryPolicy,
    /// Uploads larger than this use a resumable session.
    pub resumable_threshold: usize,
    /// Resumable chunk size (Google requires a multiple of 256 KiB).
    pub chunk_size: usize,
    /// Name of the top-level folder in Drive.
    pub root_folder: String,
}

impl DriveConfig {
    pub fn google(oauth: OAuthConfig) -> Self {
        Self {
            oauth,
            api_url: "https://www.googleapis.com/drive/v3".into(),
            upload_url: "https://www.googleapis.com/upload/drive/v3".into(),
            retry: RetryPolicy::default(),
            resumable_threshold: 5 * 1024 * 1024,
            chunk_size: 8 * 1024 * 1024,
            root_folder: ROOT_FOLDER.into(),
        }
    }
}

/// Exponential backoff with jitter for transient failures.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total tries, including the first.
    pub attempts: u32,
    pub base: Duration,
    pub max: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 5,
            base: Duration::from_millis(500),
            max: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Delay before retry number `retry` (1-based); the server's
    /// `Retry-After` wins when present.
    fn delay(&self, retry: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(after) = retry_after {
            return after.min(self.max);
        }
        let exp = self.base.saturating_mul(1 << retry.min(16)).min(self.max);
        // 50–100 % of the exponential delay, so devices don't retry in lockstep.
        let mut r = [0u8; 2];
        getrandom::fill(&mut r).expect("OS random number generator");
        let fraction = 0.5 + f64::from(u16::from_le_bytes(r)) / f64::from(u16::MAX) / 2.0;
        exp.mul_f64(fraction)
    }
}

/// Drive folder ids of the three areas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folders {
    pub root: String,
    pub objects: String,
    pub blobs: String,
}

impl Folders {
    fn id(&self, area: Area) -> &str {
        match area {
            Area::Root => &self.root,
            Area::Objects => &self.objects,
            Area::Blobs => &self.blobs,
        }
    }

    fn area(&self, folder_id: &str) -> Option<Area> {
        Area::ALL.into_iter().find(|&a| self.id(a) == folder_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub email: String,
    pub name: String,
}

pub struct DriveProvider {
    config: DriveConfig,
    http: reqwest::Client,
    /// For resumable sessions, whose "308 Resume Incomplete" must not be
    /// treated as a redirect.
    upload_http: reqwest::Client,
    refresh_token: Zeroizing<String>,
    access: Mutex<Option<AccessToken>>,
    folders: RwLock<Option<Folders>>,
}

enum Failure {
    Unauthorized,
    Retry(Option<Duration>, ProviderError),
    Fatal(ProviderError),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFile {
    id: String,
    #[serde(default)]
    name: String,
    size: Option<String>,
    modified_time: Option<String>,
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    parents: Vec<String>,
    #[serde(default)]
    trashed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    #[serde(default)]
    files: Vec<DriveFile>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeList {
    #[serde(default)]
    changes: Vec<Change>,
    next_page_token: Option<String>,
    new_start_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Change {
    file_id: String,
    #[serde(default)]
    removed: bool,
    file: Option<DriveFile>,
}

impl DriveProvider {
    pub fn new(config: DriveConfig, refresh_token: Zeroizing<String>) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            upload_http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("HTTP client"),
            refresh_token,
            access: Mutex::new(None),
            folders: RwLock::new(None),
        }
    }

    /// Uses folder ids found earlier (saves lookups on every start).
    pub fn with_folders(self, folders: Folders) -> Self {
        *self.folders.write().expect("lock") = Some(folders);
        self
    }

    /// Finds the `Memoir` folders, creating them if needed.
    pub async fn ensure_folders(&self) -> Result<Folders> {
        let root = self
            .find_or_create_folder(&self.config.root_folder, "root")
            .await?;
        let folders = Folders {
            objects: self.find_or_create_folder("objects", &root).await?,
            blobs: self.find_or_create_folder("blobs", &root).await?,
            root,
        };
        *self.folders.write().expect("lock") = Some(folders.clone());
        Ok(folders)
    }

    pub async fn account(&self) -> Result<Account> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct About {
            user: User,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct User {
            #[serde(default)]
            email_address: String,
            #[serde(default)]
            display_name: String,
        }
        let url = format!("{}/about", self.config.api_url);
        let about: About = json_body(
            self.send(|c| {
                c.get(&url)
                    .query(&[("fields", "user(emailAddress,displayName)")])
            })
            .await?,
        )
        .await?;
        Ok(Account {
            email: about.user.email_address,
            name: about.user.display_name,
        })
    }

    /// Signs out: tells Google to invalidate the refresh token.
    pub async fn revoke(&self) -> std::result::Result<(), OAuthError> {
        oauth::revoke(&self.config.oauth, &self.http, &self.refresh_token).await
    }

    fn folders(&self) -> Result<Folders> {
        self.folders
            .read()
            .expect("lock")
            .clone()
            .ok_or_else(|| ProviderError::Other("Drive folders not set up".into()))
    }

    // ---- Requests -----------------------------------------------------

    /// A valid access token, refreshed when stale (or when `force`).
    async fn token(&self, force: bool) -> Result<Zeroizing<String>> {
        let mut access = self.access.lock().await;
        if force || access.as_ref().is_none_or(AccessToken::is_stale) {
            let fresh = oauth::refresh(&self.config.oauth, &self.http, &self.refresh_token)
                .await
                .map_err(|e| match e {
                    OAuthError::Network(m) => ProviderError::Network(m),
                    OAuthError::Expired | OAuthError::Denied(_) => ProviderError::Auth,
                    other => ProviderError::Other(other.to_string()),
                })?;
            *access = Some(fresh);
        }
        Ok(access.as_ref().expect("just set").token.clone())
    }

    /// Sends with a valid token; on 401 refreshes once; retries transient
    /// failures with backoff.
    async fn send(
        &self,
        make: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        self.send_with(&self.http, make, true).await
    }

    async fn send_with(
        &self,
        client: &reqwest::Client,
        make: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder,
        retry: bool,
    ) -> Result<reqwest::Response> {
        let mut refreshed = false;
        let mut retries = 0;
        loop {
            let failure = match self.token(false).await {
                Err(e) if e.is_transient() => Failure::Retry(None, e),
                Err(e) => return Err(e),
                Ok(token) => match make(client).bearer_auth(token.as_str()).send().await {
                    Err(e) => Failure::Retry(None, ProviderError::Network(e.to_string())),
                    Ok(r) if r.status().is_success() || r.status().as_u16() == 308 => return Ok(r),
                    Ok(r) => classify(r).await,
                },
            };
            let (after, error) = match failure {
                Failure::Unauthorized if !refreshed => {
                    refreshed = true;
                    // Retry with a freshly minted token.
                    self.token(true).await?;
                    continue;
                }
                Failure::Unauthorized => return Err(ProviderError::Auth),
                Failure::Fatal(e) => return Err(e),
                Failure::Retry(after, e) => (after, e),
            };
            retries += 1;
            if !retry || retries >= self.config.retry.attempts {
                return Err(error);
            }
            log::info!("Drive request failed ({error}); retry {retries}");
            tokio::time::sleep(self.config.retry.delay(retries, after)).await;
        }
    }

    // ---- Folders --------------------------------------------------------

    async fn find_or_create_folder(&self, name: &str, parent: &str) -> Result<String> {
        let url = format!("{}/files", self.config.api_url);
        let q = format!(
            "name = '{name}' and mimeType = '{FOLDER_MIME}' and '{parent}' in parents and trashed = false"
        );
        let found: FileList = json_body(
            self.send(|c| {
                c.get(&url).query(&[
                    ("q", q.as_str()),
                    ("fields", "files(id)"),
                    // If two devices raced to create it, everyone uses the oldest.
                    ("orderBy", "createdTime"),
                ])
            })
            .await?,
        )
        .await?;
        if let Some(folder) = found.files.into_iter().next() {
            return Ok(folder.id);
        }
        let created: DriveFile = json_body(
            self.send(|c| {
                c.post(&url)
                    .query(&[("fields", "id")])
                    .json(&json!({ "name": name, "mimeType": FOLDER_MIME, "parents": [parent] }))
            })
            .await?,
        )
        .await?;
        Ok(created.id)
    }

    // ---- Uploads ----------------------------------------------------------

    async fn upload_multipart(&self, folder: &str, name: &str, data: &[u8]) -> Result<DriveFile> {
        let url = format!("{}/files", self.config.upload_url);
        let boundary = format!("memoir-{}", hex_random(12));
        let metadata = json!({ "name": name, "parents": [folder] });
        let mut body = Vec::with_capacity(data.len() + 512);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n\
                 --{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        json_body(
            self.send(|c| {
                c.post(&url)
                    .query(&[("uploadType", "multipart"), ("fields", FILE_FIELDS)])
                    .header(
                        CONTENT_TYPE,
                        format!("multipart/related; boundary={boundary}"),
                    )
                    .body(body.clone())
            })
            .await?,
        )
        .await
    }

    /// Resumable upload: chunks are sent in order; after a failure the
    /// session is asked how much arrived and the upload continues from there.
    async fn upload_resumable(&self, folder: &str, name: &str, data: &[u8]) -> Result<DriveFile> {
        let total = data.len();
        let start_url = format!("{}/files", self.config.upload_url);
        let metadata = json!({ "name": name, "parents": [folder] });

        // A session that expired (404/410) is restarted once.
        for _ in 0..2 {
            let response = self
                .send(|c| {
                    c.post(&start_url)
                        .query(&[("uploadType", "resumable"), ("fields", FILE_FIELDS)])
                        .header("X-Upload-Content-Type", "application/octet-stream")
                        .header("X-Upload-Content-Length", total.to_string())
                        .json(&metadata)
                })
                .await?;
            let session = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| ProviderError::Other("no resumable session URL".into()))?
                .to_owned();

            let mut offset = 0usize;
            let mut failures = 0u32;
            loop {
                let end = (offset + self.config.chunk_size).min(total);
                let chunk = &data[offset..end];
                let range = if total == 0 {
                    "bytes */0".to_owned()
                } else {
                    format!("bytes {offset}-{}/{total}", end - 1)
                };
                let sent = self
                    .send_with(
                        &self.upload_http,
                        |c| {
                            c.put(&session)
                                .header(CONTENT_RANGE, &range)
                                .body(chunk.to_vec())
                        },
                        false,
                    )
                    .await;
                let response = match sent {
                    Ok(r) => r,
                    Err(ProviderError::NotFound(_)) => break, // session gone: start over
                    Err(e) if e.is_transient() => {
                        failures += 1;
                        if failures >= self.config.retry.attempts {
                            return Err(e);
                        }
                        tokio::time::sleep(self.config.retry.delay(failures, None)).await;
                        match self.upload_status(&session, total).await? {
                            UploadStatus::Done(file) => return Ok(file),
                            UploadStatus::Received(next) => offset = next,
                            UploadStatus::Expired => break,
                        }
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                if response.status().as_u16() == 308 {
                    offset = next_offset(&response);
                    continue;
                }
                return json_body(response).await;
            }
        }
        Err(ProviderError::Other("upload session kept expiring".into()))
    }

    async fn upload_status(&self, session: &str, total: usize) -> Result<UploadStatus> {
        let range = format!("bytes */{total}");
        match self
            .send_with(
                &self.upload_http,
                |c| {
                    c.put(session)
                        .header(CONTENT_RANGE, &range)
                        .body(Vec::new())
                },
                true,
            )
            .await
        {
            Ok(r) if r.status().as_u16() == 308 => Ok(UploadStatus::Received(next_offset(&r))),
            Ok(r) => Ok(UploadStatus::Done(json_body(r).await?)),
            Err(ProviderError::NotFound(_)) => Ok(UploadStatus::Expired),
            Err(e) => Err(e),
        }
    }
}

enum UploadStatus {
    Done(DriveFile),
    Received(usize),
    Expired,
}

/// Next byte to send after a 308: `Range: bytes=0-N` means N+1 bytes arrived.
fn next_offset(response: &reqwest::Response) -> usize {
    response
        .headers()
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('-').next())
        .and_then(|n| n.parse::<usize>().ok())
        .map_or(0, |last| last + 1)
}

impl StorageProvider for DriveProvider {
    async fn list(&self, area: Area) -> Result<Vec<RemoteFile>> {
        let folder = self.folders()?.id(area).to_owned();
        let url = format!("{}/files", self.config.api_url);
        let q = format!("'{folder}' in parents and trashed = false");
        let fields = format!("nextPageToken,files({FILE_FIELDS})");
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let list: FileList = json_body(
                self.send(|c| {
                    let mut query = vec![
                        ("q", q.as_str()),
                        ("fields", fields.as_str()),
                        ("pageSize", "1000"),
                    ];
                    if let Some(p) = &page {
                        query.push(("pageToken", p));
                    }
                    c.get(&url).query(&query)
                })
                .await?,
            )
            .await?;
            out.extend(list.files.into_iter().map(|f| remote_file(f, area)));
            match list.next_page_token {
                Some(next) => page = Some(next),
                None => return Ok(out),
            }
        }
    }

    async fn cursor(&self) -> Result<String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Start {
            start_page_token: String,
        }
        let url = format!("{}/changes/startPageToken", self.config.api_url);
        let start: Start = json_body(self.send(|c| c.get(&url)).await?).await?;
        Ok(start.start_page_token)
    }

    async fn changes(&self, cursor: &str) -> Result<Changes> {
        let folders = self.folders()?;
        let url = format!("{}/changes", self.config.api_url);
        let fields = "nextPageToken,newStartPageToken,\
                      changes(fileId,removed,file(id,name,size,modifiedTime,mimeType,parents,trashed))";
        // Later changes to the same file replace earlier ones.
        let mut latest: Vec<(String, Option<RemoteFile>)> = Vec::new();
        let mut page = cursor.to_owned();
        loop {
            let list: ChangeList = json_body(
                self.send(|c| {
                    c.get(&url).query(&[
                        ("pageToken", page.as_str()),
                        ("fields", fields),
                        ("pageSize", "1000"),
                        ("includeRemoved", "true"),
                        ("spaces", "drive"),
                    ])
                })
                .await?,
            )
            .await?;
            for change in list.changes {
                let entry = match change.file {
                    Some(f) if !change.removed && !f.trashed => {
                        if f.mime_type == FOLDER_MIME {
                            continue;
                        }
                        let Some(area) = f.parents.iter().find_map(|p| folders.area(p)) else {
                            continue; // not one of ours
                        };
                        Some(remote_file(f, area))
                    }
                    _ => None,
                };
                latest.retain(|(id, _)| id != &change.file_id);
                latest.push((change.file_id, entry));
            }
            if let Some(next) = list.new_start_page_token {
                let mut changes = Changes {
                    cursor: next,
                    ..Changes::default()
                };
                for (id, file) in latest {
                    match file {
                        Some(f) => changes.files.push(f),
                        None => changes.removed.push(id),
                    }
                }
                return Ok(changes);
            }
            page = list
                .next_page_token
                .ok_or_else(|| ProviderError::Other("changes list without a next page".into()))?;
        }
    }

    async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let url = format!("{}/files/{file_id}", self.config.api_url);
        let response = self
            .send(|c| c.get(&url).query(&[("alt", "media")]))
            .await?;
        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| ProviderError::Network(e.to_string()))
    }

    async fn upload(&self, area: Area, name: &str, data: Vec<u8>) -> Result<RemoteFile> {
        let folder = self.folders()?.id(area).to_owned();
        let file = if data.len() > self.config.resumable_threshold {
            self.upload_resumable(&folder, name, &data).await?
        } else {
            self.upload_multipart(&folder, name, &data).await?
        };
        Ok(remote_file(file, area))
    }

    async fn delete(&self, file_id: &str) -> Result<()> {
        let url = format!("{}/files/{file_id}", self.config.api_url);
        self.send(|c| c.delete(&url)).await?;
        Ok(())
    }
}

fn remote_file(f: DriveFile, area: Area) -> RemoteFile {
    RemoteFile {
        size: f.size.and_then(|s| s.parse().ok()).unwrap_or(0),
        modified: f.modified_time.as_deref().and_then(rfc3339_ms).unwrap_or(0),
        id: f.id,
        name: f.name,
        area,
    }
}

async fn json_body<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let bytes = response
        .bytes()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| ProviderError::Other(format!("unexpected response: {e}")))
}

/// Turns an error response into what to do about it.
async fn classify(response: reqwest::Response) -> Failure {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = response.text().await.unwrap_or_default();
    let reason = error_reason(&body);
    match status {
        401 => Failure::Unauthorized,
        403 => match reason.as_str() {
            "rateLimitExceeded" | "userRateLimitExceeded" => {
                Failure::Retry(retry_after, ProviderError::Busy)
            }
            "storageQuotaExceeded" | "quotaExceeded" if body.contains("storage") => {
                Failure::Fatal(ProviderError::Quota)
            }
            "insufficientPermissions" | "authError" => Failure::Fatal(ProviderError::Auth),
            _ => Failure::Fatal(ProviderError::Other(format!("access denied ({reason})"))),
        },
        404 | 410 => Failure::Fatal(ProviderError::NotFound(reason)),
        429 | 500..=599 => Failure::Retry(retry_after, ProviderError::Busy),
        _ => Failure::Fatal(ProviderError::Other(format!("HTTP {status}: {reason}"))),
    }
}

/// `error.errors[0].reason` from a Drive error body (or `error.status`).
fn error_reason(body: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return String::new();
    };
    v["error"]["errors"][0]["reason"]
        .as_str()
        .or_else(|| v["error"]["status"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// Unix milliseconds of an RFC 3339 UTC timestamp as Drive writes them
/// (`2026-10-25T19:07:39.123Z`).
fn rfc3339_ms(s: &str) -> Option<i64> {
    let (date, time) = s.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = hms.split(':').map(|p| p.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    let ms: i64 = format!("{frac:0<3}")[..3].parse().ok()?;
    let days = days_from_civil(y, m, day);
    Some((((days * 24 + hh) * 60 + mm) * 60 + ss) * 1000 + ms)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn hex_random(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random number generator");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_drive_timestamps() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(rfc3339_ms("1970-01-02T00:00:01Z"), Some(86_401_000));
        assert_eq!(
            rfc3339_ms("2000-03-01T00:00:00.5Z"),
            Some(11_017 * 86_400_000 + 500)
        );
        assert_eq!(
            rfc3339_ms("2026-10-25T19:07:39.123Z"),
            Some(1_792_955_259_123)
        );
        assert_eq!(rfc3339_ms("garbage"), None);
    }

    #[test]
    fn backoff_grows_and_respects_retry_after() {
        let p = RetryPolicy {
            attempts: 5,
            base: Duration::from_millis(100),
            max: Duration::from_secs(1),
        };
        let d1 = p.delay(1, None);
        assert!(d1 >= Duration::from_millis(100) && d1 <= Duration::from_millis(200));
        assert!(p.delay(10, None) <= Duration::from_secs(1));
        assert_eq!(
            p.delay(1, Some(Duration::from_secs(3))),
            Duration::from_secs(1)
        );
        assert_eq!(p.delay(1, Some(Duration::ZERO)), Duration::ZERO);
    }
}

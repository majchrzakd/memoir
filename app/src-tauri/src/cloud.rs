//! The Google Drive connection: connect (browser sign-in), check, disconnect,
//! restoring the connection when the vault unlocks, and opening a notebook
//! that's already in Drive on a new device.
//!
//! The OAuth client (from Google Cloud Console, see docs/spikes/google-oauth.md)
//! is taken from `MEMOIR_GOOGLE_CLIENT_ID` / `MEMOIR_GOOGLE_CLIENT_SECRET`,
//! at build time or (for development) at run time. iOS uses its own client
//! (`IOS_CLIENT_ID`), which comes back to the app through its URL scheme.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use memoir_core::remote::{Area, ProviderError, RemoteFile, StorageProvider};
use memoir_core::sync::SyncEngine;
use memoir_core::vault::VaultFile;
use memoir_gdrive::{DriveConfig, DriveProvider, Folders, OAuthConfig, OAuthError, oauth};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Notify;
use zeroize::Zeroizing;

use crate::commands::AppState;
use crate::workspaces::Workspaces;

const VAULT_FILE: &str = "vault.json";
const SWITCHED: &str = "Another workspace was opened meanwhile.";

/// The Google OAuth client for iOS (type "iOS", bundle ID
/// `com.memoir.notebook`). Not a secret: it ships in every copy of the app,
/// and its reversed form is the URL scheme registered in `Info.ios.plist`.
#[cfg(target_os = "ios")]
const IOS_CLIENT_ID: &str =
    "562411400187-f7ifiiin7d0uqqdcpqih6fie7o96lps1.apps.googleusercontent.com";
const PROVIDER: &str = "gdrive";

/// Keys in the store's `remote_state`.
mod key {
    pub const PROVIDER: &str = "provider";
    pub const ACCOUNT: &str = "account";
    pub const FOLDERS: &str = "folders";
}

#[derive(Default)]
pub struct Cloud {
    provider: RwLock<Option<Arc<DriveProvider>>>,
    cancel: Notify,
    connecting: AtomicBool,
    /// Signed in on a new device, notebook found, waiting for the passphrase.
    joining: Mutex<Option<Joining>>,
    /// Workspace switches so far. A sign-in belongs to the notebook that was
    /// open when it started; one that ends after a switch is dropped.
    generation: AtomicU64,
    /// iOS: where to pass the URLs Google's sign-in redirects back to, while a
    /// sign-in is waiting.
    #[cfg(target_os = "ios")]
    redirect: Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
    /// iOS: a sign-in came back with nothing waiting for it (iOS restarted
    /// the app while the user was in Safari, or it was cancelled).
    #[cfg(target_os = "ios")]
    redirect_lost: AtomicBool,
}

/// A Google sign-in with the Memoir folder set up.
struct SignedIn {
    provider: DriveProvider,
    folders: Folders,
    account: String,
    refresh_token: Zeroizing<String>,
    /// [`Cloud::generation`] when the sign-in started.
    generation: u64,
}

impl SignedIn {
    /// Gives up on this sign-in: Memoir's access is revoked in the background.
    fn revoke_later(self) {
        tauri::async_runtime::spawn(async move {
            let _ = self.provider.revoke().await;
        });
    }
}

struct Joining {
    signed_in: SignedIn,
    vault: VaultFile,
}

impl Cloud {
    /// iOS opened the app with `url`: the end of a Google sign-in (other URLs
    /// are ignored).
    #[cfg(target_os = "ios")]
    pub fn opened(&self, url: &str) {
        if !url.starts_with(&oauth::app_redirect_uri(IOS_CLIENT_ID)) {
            return;
        }
        match &*self.redirect.lock().expect("lock") {
            Some(waiting) => {
                let _ = waiting.send(url.to_owned());
            }
            None => self.redirect_lost.store(true, Ordering::SeqCst),
        }
    }

    /// Something to tell the user about the last sign-in, if any.
    fn sign_in_problem(&self) -> Option<String> {
        #[cfg(target_os = "ios")]
        if self.redirect_lost.load(Ordering::SeqCst) {
            return Some(
                "Google sign-in finished after Memoir stopped waiting for it. Connect again."
                    .into(),
            );
        }
        None
    }

    /// The connected provider, if any.
    pub fn provider(&self) -> Option<Arc<DriveProvider>> {
        self.provider.read().expect("lock").clone()
    }

    /// Another notebook was opened (workspace switch): drops this one's
    /// connection and any sign-in in progress.
    pub fn reset(&self) {
        // First, so a sign-in finishing meanwhile sees it (see `find_drive_notebook`).
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel.notify_waiters();
        self.set(None);
        if let Some(joining) = self.joining.lock().expect("lock").take() {
            joining.signed_in.revoke_later();
        }
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Whether `signed_in` started in the notebook that's open now.
    fn current(&self, signed_in: &SignedIn) -> bool {
        signed_in.generation == self.generation()
    }

    fn set(&self, provider: Option<DriveProvider>) {
        *self.provider.write().expect("lock") = provider.map(Arc::new);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudStatus {
    /// This build knows a Google OAuth client.
    pub configured: bool,
    pub connected: bool,
    pub connecting: bool,
    pub account: Option<String>,
    /// Something the user should know or do (e.g. sign in again).
    pub problem: Option<String>,
}

#[cfg(target_os = "ios")]
fn oauth_config() -> Option<OAuthConfig> {
    Some(OAuthConfig::google(IOS_CLIENT_ID, ""))
}

#[cfg(not(target_os = "ios"))]
fn oauth_config() -> Option<OAuthConfig> {
    let id = option_env!("MEMOIR_GOOGLE_CLIENT_ID")
        .map(str::to_owned)
        .or_else(|| std::env::var("MEMOIR_GOOGLE_CLIENT_ID").ok())?;
    let secret = option_env!("MEMOIR_GOOGLE_CLIENT_SECRET")
        .map(str::to_owned)
        .or_else(|| std::env::var("MEMOIR_GOOGLE_CLIENT_SECRET").ok())
        .unwrap_or_default();
    (!id.trim().is_empty()).then(|| OAuthConfig::google(id.trim(), secret.trim()))
}

/// Recreates the provider from saved state after the vault unlocks. No
/// network access; problems surface on the next check or sync.
pub fn restore(state: &AppState) {
    let restored = (|| -> Option<DriveProvider> {
        let config = oauth_config()?;
        let mut session = state.session.lock().ok()?;
        let store = session.store().ok()?;
        if store.remote_value(key::PROVIDER).ok()?.as_deref() != Some(PROVIDER) {
            return None;
        }
        let folders: Folders =
            serde_json::from_str(&store.remote_value(key::FOLDERS).ok()??).ok()?;
        let token = session.cloud_token().ok()??;
        Some(DriveProvider::new(DriveConfig::google(config), token).with_folders(folders))
    })();
    state.cloud.set(restored);
}

fn status(state: &AppState, problem: Option<String>) -> CloudStatus {
    let account = state
        .session
        .lock()
        .ok()
        .and_then(|mut s| s.store().ok()?.remote_value(key::ACCOUNT).ok().flatten());
    let saved = account.is_some();
    let connected = state.cloud.provider().is_some();
    CloudStatus {
        configured: oauth_config().is_some(),
        connected,
        connecting: state.cloud.connecting.load(Ordering::SeqCst),
        account,
        problem: problem
            .or_else(|| (!connected).then(|| state.cloud.sign_in_problem())?)
            .or_else(|| {
                (saved && !connected)
                    .then(|| "The Google sign-in is missing on this device. Connect again.".into())
            }),
    }
}

#[tauri::command(async)]
pub fn cloud_status(state: State<'_, AppState>) -> CloudStatus {
    status(&state, None)
}

/// Verifies the connection with a small request.
#[tauri::command]
pub async fn check_drive(state: State<'_, AppState>) -> Result<CloudStatus, String> {
    let Some(provider) = state.cloud.provider() else {
        return Ok(status(&state, None));
    };
    let problem = match provider.account().await {
        Ok(_) => None,
        Err(ProviderError::Auth) => Some(
            "Google no longer accepts Memoir's sign-in (it expired or was revoked). Connect again."
                .into(),
        ),
        Err(e) => Some(e.to_string()),
    };
    Ok(status(&state, problem))
}

/// Browser sign-in, then sets up the Memoir folder in Drive.
#[tauri::command]
pub async fn connect_drive<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<CloudStatus, String> {
    let config = oauth_config().ok_or("this build has no Google client configured")?;
    if state.cloud.connecting.swap(true, Ordering::SeqCst) {
        return Err("already connecting".into());
    }
    let result = connect(&app, &state, config).await;
    state.cloud.connecting.store(false, Ordering::SeqCst);
    if result.is_ok() {
        state.sync.now();
    }
    result.map(|()| status(&state, None))
}

async fn connect<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    config: OAuthConfig,
) -> Result<(), String> {
    let local_vault = state
        .session()
        .and_then(|s| s.vault_file().map_err(|e| e.to_string()))?;
    let signed_in = sign_in(app, state, config).await?;

    // One notebook per Drive: refuse a Drive that holds a different one.
    match remote_vault(&signed_in.provider).await? {
        Some(remote) if remote.vault_id != local_vault.vault_id => {
            let _ = signed_in.provider.revoke().await;
            return Err(format!(
                "The Memoir folder in {} already holds a different notebook. Use another Google \
                 account, or remove that Memoir folder from Drive.",
                signed_in.account
            ));
        }
        Some(_) => {}
        None => {
            signed_in
                .provider
                .upload(Area::Root, VAULT_FILE, local_vault.to_json().into_bytes())
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    remember(state, signed_in)
}

/// Browser sign-in, then finds (or creates) the Memoir folder.
async fn sign_in<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    config: OAuthConfig,
) -> Result<SignedIn, String> {
    let generation = state.cloud.generation();
    let http = reqwest::Client::new();
    let open_browser = |url: &str| {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())
    };
    #[cfg(not(target_os = "ios"))]
    let sign_in = oauth::sign_in(&config, &http, open_browser, state.cloud.cancel.notified()).await;
    #[cfg(target_os = "ios")]
    let sign_in = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.cloud.redirect_lost.store(false, Ordering::SeqCst);
        *state.cloud.redirect.lock().expect("lock") = Some(tx);
        let sign_in = oauth::sign_in_with_app_redirect(
            &config,
            &http,
            &oauth::app_redirect_uri(&config.client_id),
            open_browser,
            rx,
            state.cloud.cancel.notified(),
        )
        .await;
        *state.cloud.redirect.lock().expect("lock") = None;
        sign_in
    };
    let sign_in = sign_in.map_err(|e| match e {
        OAuthError::Cancelled => "Connecting was cancelled.".to_string(),
        other => other.to_string(),
    })?;

    let provider = DriveProvider::new(DriveConfig::google(config), sign_in.refresh_token.clone());
    let folders = provider.ensure_folders().await.map_err(|e| e.to_string())?;
    let account = provider.account().await.map_err(|e| e.to_string())?;
    Ok(SignedIn {
        provider: provider.with_folders(folders.clone()),
        folders,
        account: account.email,
        refresh_token: Zeroizing::new(sign_in.refresh_token.to_string()),
        generation,
    })
}

/// The notebook's vault file in Drive, if there is one.
async fn remote_vault(provider: &DriveProvider) -> Result<Option<VaultFile>, String> {
    let copies = remote_vaults(provider).await?;
    if copies.is_empty() {
        return Ok(None);
    }
    // Normally one; while a change is being uploaded there can briefly be two.
    copies
        .into_iter()
        .filter_map(|(_, file)| file)
        .max_by_key(|f| f.updated_at.unwrap_or(0))
        .map(Some)
        .ok_or_else(|| "The notebook file in Drive can't be read.".to_string())
}

/// Every `vault.json` in the Memoir folder, with its content (None if
/// unreadable).
async fn remote_vaults(
    provider: &DriveProvider,
) -> Result<Vec<(RemoteFile, Option<VaultFile>)>, String> {
    let files = provider.list(Area::Root).await.map_err(|e| e.to_string())?;
    let mut copies = Vec::new();
    for file in files.into_iter().filter(|f| f.name == VAULT_FILE) {
        let json = provider
            .download(&file.id)
            .await
            .map_err(|e| e.to_string())?;
        let parsed = VaultFile::from_json(&String::from_utf8_lossy(&json)).ok();
        copies.push((file, parsed));
    }
    Ok(copies)
}

/// Keeps `vault.json` in Drive and on this device in step. A signed, newer
/// copy in Drive (passphrase or recovery key changed on another device)
/// replaces this device's; a signed, newer copy here (changed on this
/// device) replaces Drive's; a missing one is uploaded. Anything unsigned
/// or older is never taken over (see `VaultFile::supersedes`).
async fn reconcile_vault(state: &AppState, provider: &DriveProvider) -> Result<(), String> {
    let remote = remote_vaults(provider).await?;
    let (local, keys) = {
        let session = state.session()?;
        (
            session.vault_file().map_err(|e| e.to_string())?,
            session.keys().map_err(|e| e.to_string())?,
        )
    };

    let newer = remote
        .iter()
        .filter_map(|(_, file)| file.as_ref())
        .filter(|file| file.supersedes(&local, &keys))
        .max_by_key(|file| file.updated_at.unwrap_or(0));
    if let Some(newer) = newer {
        state
            .session()?
            .adopt_vault_file(newer)
            .map_err(|e| e.to_string())?;
        log::info!(
            "the passphrase or recovery key was changed on another device; using the new one"
        );
        return Ok(());
    }

    let in_step = remote.iter().any(|(_, file)| file.as_ref() == Some(&local));
    let ours_is_newest = remote
        .iter()
        .all(|(_, file)| file.as_ref().is_none_or(|f| local.supersedes(f, &keys)));
    if in_step || !ours_is_newest {
        return Ok(());
    }
    if remote.is_empty() {
        log::warn!("vault.json was missing in Drive; uploading it again");
    } else {
        log::info!("uploading this device's new passphrase / recovery key setup");
    }
    provider
        .upload(Area::Root, VAULT_FILE, local.to_json().into_bytes())
        .await
        .map_err(|e| e.to_string())?;
    for (old, _) in &remote {
        if let Err(e) = provider.delete(&old.id).await {
            log::warn!("could not remove the old vault.json from Drive: {e}");
        }
    }
    Ok(())
}

/// Saves the sign-in for this (unlocked) notebook and starts using it.
fn remember(state: &AppState, signed_in: SignedIn) -> Result<(), String> {
    {
        let mut session = state.session()?;
        // The session is replaced only after `Cloud::reset`, and not while
        // this lock is held.
        if !state.cloud.current(&signed_in) {
            signed_in.revoke_later();
            return Err(SWITCHED.into());
        }
        session
            .set_cloud_token(&signed_in.refresh_token)
            .map_err(|e| e.to_string())?;
        let folders_json = serde_json::to_string(&signed_in.folders).expect("folders serialize");
        session
            .store()
            .and_then(|s| {
                s.set_remote_values(&[
                    (key::PROVIDER, PROVIDER),
                    (key::ACCOUNT, &signed_in.account),
                    (key::FOLDERS, &folders_json),
                ])
                .map_err(Into::into)
            })
            .map_err(|e| e.to_string())?;
    }
    state.cloud.set(Some(signed_in.provider));
    Ok(())
}

/// Makes sure the Memoir folders and `vault.json` are still in Drive — the
/// user may have deleted or trashed them — and recreates what's missing;
/// also brings passphrase / recovery key changes across devices.
/// Runs at the start of a session, after unlocking and after such a change.
/// The next sync round then lists the cloud fully and uploads every note
/// again (see `Store::enqueue_lost`). Run once per session.
pub async fn check_cloud_folder(state: &AppState, provider: &DriveProvider) -> Result<(), String> {
    let folders = provider.ensure_folders().await.map_err(|e| e.to_string())?;
    let changed = {
        let mut session = state.session()?;
        let store = session.store().map_err(|e| e.to_string())?;
        let stored: Option<Folders> = store
            .remote_value(key::FOLDERS)
            .map_err(|e| e.to_string())?
            .and_then(|json| serde_json::from_str(&json).ok());
        let changed = stored.as_ref() != Some(&folders);
        if changed {
            let json = serde_json::to_string(&folders).expect("folders serialize");
            store
                .set_remote_values(&[(key::FOLDERS, &json)])
                .map_err(|e| e.to_string())?;
            SyncEngine::<DriveProvider>::list_fully_next_time(store).map_err(|e| e.to_string())?;
        }
        changed
    };
    if changed {
        log::warn!("the Memoir folder in Drive was missing; it was created again");
    }
    reconcile_vault(state, provider).await
}

/// New device, step 1: sign in and find the notebook in Drive. Returns the
/// Google account it was found in.
#[tauri::command]
pub async fn find_drive_notebook<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let config = oauth_config().ok_or("this build has no Google client configured")?;
    if state.cloud.connecting.swap(true, Ordering::SeqCst) {
        return Err("already connecting".into());
    }
    let result = async {
        let signed_in = sign_in(&app, &state, config).await?;
        match remote_vault(&signed_in.provider).await? {
            Some(vault) => {
                let workspaces = app.state::<Workspaces>();
                if let Some(other) = workspaces.holding(&vault.vault_id, &workspaces.active()) {
                    let _ = signed_in.provider.revoke().await;
                    return Err(format!(
                        "The notebook in the Google Drive of {} is already on this device, in the \
                         workspace \"{}\".",
                        signed_in.account, other.name
                    ));
                }
                Ok(Joining { signed_in, vault })
            }
            None => {
                let account = signed_in.account.clone();
                let _ = signed_in.provider.revoke().await;
                Err(format!(
                    "There's no Memoir notebook in the Google Drive of {account}. Sign in with \
                     the account your notebook is in, or create a new notebook."
                ))
            }
        }
    }
    .await;
    state.cloud.connecting.store(false, Ordering::SeqCst);
    let joining = result?;
    let account = joining.signed_in.account.clone();
    let mut waiting = state.cloud.joining.lock().expect("lock");
    // `Cloud::reset` counts the switch before taking what's waiting here.
    if !state.cloud.current(&joining.signed_in) {
        joining.signed_in.revoke_later();
        return Err(SWITCHED.into());
    }
    *waiting = Some(joining);
    Ok(account)
}

/// New device, step 2: unlock the notebook found in step 1 with its
/// passphrase. Its notes then arrive by sync.
#[tauri::command]
pub async fn open_drive_notebook<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    passphrase: String,
    remember_key: bool,
) -> Result<(), String> {
    let joining = state
        .cloud
        .joining
        .lock()
        .expect("lock")
        .take()
        .ok_or("sign in to Google first")?;
    // Unlocking takes a moment (key derivation); keep it off the async threads.
    let vault = joining.vault.clone();
    let generation = joining.signed_in.generation;
    let passphrase = Zeroizing::new(passphrase);
    let handle = app.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let mut session = state.session()?;
        if state.cloud.generation() != generation {
            return Err(SWITCHED.to_string());
        }
        session
            .join(&vault, &passphrase, remember_key)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Err(e) = joined {
        // Wrong passphrase: keep the sign-in for another try.
        let mut waiting = state.cloud.joining.lock().expect("lock");
        if state.cloud.current(&joining.signed_in) {
            *waiting = Some(joining);
        } else {
            joining.signed_in.revoke_later();
        }
        return Err(e);
    }
    remember(&state, joining.signed_in)?;
    state.sync.now();
    Ok(())
}

/// New device: gives up on the notebook found in Drive.
#[tauri::command]
pub async fn forget_drive_notebook(state: State<'_, AppState>) -> Result<(), String> {
    let joining = state.cloud.joining.lock().expect("lock").take();
    if let Some(j) = joining {
        let _ = j.signed_in.provider.revoke().await;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn cancel_connect(state: State<'_, AppState>) {
    state.cloud.cancel.notify_waiters();
}

/// Forgets the connection on this device and revokes Memoir's access.
/// Files already in Drive stay there.
#[tauri::command]
pub async fn disconnect_drive(state: State<'_, AppState>) -> Result<CloudStatus, String> {
    if let Some(provider) = state.cloud.provider()
        && let Err(e) = provider.revoke().await
    {
        log::warn!("could not revoke the Google token: {e}");
    }
    state.cloud.set(None);
    state.sync.now(); // reports "off"
    {
        let mut session = state.session()?;
        session.forget_cloud_token().map_err(|e| e.to_string())?;
        session
            .store()
            .and_then(|s| s.clear_remote_state().map_err(Into::into))
            .map_err(|e| e.to_string())?;
    }
    Ok(status(&state, None))
}

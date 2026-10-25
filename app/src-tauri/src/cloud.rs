//! The Google Drive connection: connect (browser sign-in), check, disconnect,
//! and restoring the connection when the vault unlocks.
//!
//! The OAuth client (from Google Cloud Console, see docs/spikes/google-oauth.md)
//! is taken from `MEMOIR_GOOGLE_CLIENT_ID` / `MEMOIR_GOOGLE_CLIENT_SECRET`,
//! at build time or (for development) at run time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use memoir_core::remote::{Area, ProviderError, StorageProvider};
use memoir_core::vault::VaultFile;
use memoir_gdrive::{DriveConfig, DriveProvider, Folders, OAuthConfig, OAuthError, oauth};
use serde::Serialize;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Notify;

use crate::commands::AppState;

const VAULT_FILE: &str = "vault.json";
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
}

impl Cloud {
    /// The connected provider, if any (used by sync in M6).
    pub fn provider(&self) -> Option<Arc<DriveProvider>> {
        self.provider.read().expect("lock").clone()
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
        problem: problem.or_else(|| {
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

    let http = reqwest::Client::new();
    let sign_in = oauth::sign_in(
        &config,
        &http,
        |url| {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| e.to_string())
        },
        state.cloud.cancel.notified(),
    )
    .await
    .map_err(|e| match e {
        OAuthError::Cancelled => "Connecting was cancelled.".to_string(),
        other => other.to_string(),
    })?;

    let provider = DriveProvider::new(DriveConfig::google(config), sign_in.refresh_token.clone());
    let folders = provider.ensure_folders().await.map_err(|e| e.to_string())?;
    let account = provider.account().await.map_err(|e| e.to_string())?;

    // One notebook per Drive: refuse a Drive that holds a different one.
    let remote_vaults: Vec<_> = provider
        .list(Area::Root)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|f| f.name == VAULT_FILE)
        .collect();
    if let Some(existing) = remote_vaults.first() {
        let json = provider
            .download(&existing.id)
            .await
            .map_err(|e| e.to_string())?;
        let remote =
            VaultFile::from_json(&String::from_utf8_lossy(&json)).map_err(|e| e.to_string())?;
        if remote.vault_id != local_vault.vault_id {
            let _ = provider.revoke().await;
            return Err(format!(
                "The Memoir folder in {} already holds a different notebook. Opening an existing \
                 notebook on a new device comes with sync; for now, use another Google account or \
                 remove that Memoir folder from Drive.",
                account.email
            ));
        }
    } else {
        provider
            .upload(Area::Root, VAULT_FILE, local_vault.to_json().into_bytes())
            .await
            .map_err(|e| e.to_string())?;
    }

    {
        let mut session = state.session()?;
        session
            .set_cloud_token(&sign_in.refresh_token)
            .map_err(|e| e.to_string())?;
        let folders_json = serde_json::to_string(&folders).expect("folders serialize");
        session
            .store()
            .and_then(|s| {
                s.set_remote_values(&[
                    (key::PROVIDER, PROVIDER),
                    (key::ACCOUNT, &account.email),
                    (key::FOLDERS, &folders_json),
                ])
                .map_err(Into::into)
            })
            .map_err(|e| e.to_string())?;
    }
    state.cloud.set(Some(provider.with_folders(folders)));
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

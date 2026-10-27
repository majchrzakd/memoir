//! Updates from GitHub Releases (Tauri updater; every update is signed with
//! the project's update key, whose public half is in tauri.conf.json).
//!
//! Memoir checks a little after start and then every 12 hours, and tells
//! the UI (`update-available`). Installing happens only when the user
//! clicks: first the editor saves and pending changes are pushed (as when
//! closing), then the installer runs and restarts Memoir.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::AppState;

pub const AVAILABLE_EVENT: &str = "update-available";
const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(12 * 3600);

#[derive(Default)]
pub struct Updates {
    /// The update found by the last check, ready to install.
    pending: Mutex<Option<Update>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    /// Release notes, if the release has any.
    pub notes: Option<String>,
}

fn info(update: &Update) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        notes: update.body.clone().filter(|b| !b.trim().is_empty()),
    }
}

/// Checks in the background now and then (release builds only: development
/// builds would offer to replace themselves with the installed version; and
/// not on iOS, where the App Store or TestFlight updates the app).
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    if cfg!(debug_assertions) || cfg!(mobile) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            match check(&app).await {
                Ok(Some(found)) => {
                    log::info!("update: {} is available", found.version);
                    let _ = app.emit(AVAILABLE_EVENT, found);
                }
                Ok(None) => {}
                // Offline, GitHub unreachable…: try again next time.
                Err(e) => log::warn!("update check failed: {e}"),
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

async fn check<R: Runtime>(app: &AppHandle<R>) -> Result<Option<UpdateInfo>, String> {
    if cfg!(mobile) {
        return Ok(None);
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let found = update.as_ref().map(info);
    *app.state::<Updates>().pending.lock().expect("lock") = update;
    Ok(found)
}

/// Checks now ("Check for updates" in Settings).
#[tauri::command]
pub async fn check_for_update<R: Runtime>(app: AppHandle<R>) -> Result<Option<UpdateInfo>, String> {
    check(&app).await
}

/// The update found earlier, if any (for a window opened after the check).
#[tauri::command(async)]
pub fn available_update(updates: State<'_, Updates>) -> Option<UpdateInfo> {
    updates.pending.lock().expect("lock").as_ref().map(info)
}

/// Pushes pending changes, then downloads and runs the installer, which
/// closes and restarts Memoir. The UI saves the editor before calling this.
#[tauri::command]
pub async fn install_update<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let Some(update) = app.state::<Updates>().pending.lock().expect("lock").take() else {
        return Err("no update to install; check for updates first".into());
    };
    log::info!("update: installing {}", update.version);
    if tokio::time::timeout(Duration::from_secs(15), crate::sync::flush(&app))
        .await
        .is_err()
    {
        log::warn!("update: sync did not finish; remaining changes stay queued");
    }
    if let Ok(session) = state.session.lock() {
        session.clear_opened_files();
    }
    let result = update
        .download_and_install(|_, _| {}, || log::info!("update: downloaded"))
        .await;
    if let Err(e) = result {
        // Offer it again.
        *app.state::<Updates>().pending.lock().expect("lock") = Some(update);
        return Err(format!("the update couldn't be installed: {e}"));
    }
    // On Windows the installer has already closed Memoir; elsewhere restart.
    app.restart();
}

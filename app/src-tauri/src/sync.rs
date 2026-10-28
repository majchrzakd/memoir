//! Background sync: one round at a time, started
//! - right away when the notebook unlocks, Drive gets connected, the window
//!   gets focus (at most every 15 s) or the user asks;
//! - a few seconds after local changes stop (so typing isn't interrupted),
//!   but at most 30 s after the first unsynced change;
//! - every minute otherwise, to pick up other devices' changes and to retry
//!   after a failure.
//!
//! The UI gets `sync-status` events (see [`SyncStatus`]) and
//! `sync-changed` events (ids of notes and folders that changed locally
//! because of other devices) to reload what it shows.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use memoir_core::blobs::BlobStore;
use memoir_core::remote::ProviderError;
use memoir_core::store::{ObjectId, Store};
use memoir_core::sync::{Local, SyncConfig, SyncEngine, SyncError, SyncReport};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tokio::sync::Notify;

use crate::commands::AppState;

pub const STATUS_EVENT: &str = "sync-status";
pub const CHANGED_EVENT: &str = "sync-changed";

const EVERY: Duration = Duration::from_secs(60);
/// Quiet time after the last local change before syncing it.
const SETTLE: Duration = Duration::from_secs(3);
/// Longest a local change waits while edits keep coming.
const MAX_DELAY: Duration = Duration::from_secs(30);
const FOCUS_MIN_GAP: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// Not connected to a cloud (or locked).
    #[default]
    Off,
    Idle,
    Syncing,
    /// The last round failed for a passing reason (no network, Google busy).
    Offline,
    /// The last round failed in a way that needs the user (sign in again).
    Error,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SyncStatus {
    pub state: SyncState,
    /// Local changes not yet in the cloud.
    pub pending: u64,
    /// Unix ms of the last successful round.
    pub last_synced_at: Option<i64>,
    pub message: Option<String>,
}

#[derive(Default)]
pub struct Sync {
    /// Something to sync right away.
    now: Notify,
    /// Local changes to sync once editing pauses.
    changed: Notify,
    /// One round at a time (background, close, manual).
    round: tokio::sync::Mutex<()>,
    status: Mutex<SyncStatus>,
    last_focus_sync: Mutex<Option<Instant>>,
    /// The Drive folders were checked this session (see `cloud::check_cloud_folder`).
    folders_checked: AtomicBool,
    started: AtomicBool,
}

impl Sync {
    /// Sync as soon as possible.
    pub fn now(&self) {
        self.now.notify_one();
    }

    /// Local data changed; sync once editing pauses.
    pub fn changed(&self) {
        self.changed.notify_one();
    }

    /// Checks the Drive folder and `vault.json` again next round (after
    /// unlocking, or a passphrase / recovery key change), and syncs now.
    pub fn recheck_cloud(&self) {
        self.folders_checked.store(false, Ordering::SeqCst);
        self.now();
    }

    pub fn focused(&self) {
        let mut last = self.last_focus_sync.lock().expect("lock");
        if last.is_none_or(|t| t.elapsed() >= FOCUS_MIN_GAP) {
            *last = Some(Instant::now());
            self.now();
        }
    }

    pub fn status(&self) -> SyncStatus {
        self.status.lock().expect("lock").clone()
    }
}

/// The engine's view of this device's data.
struct AppLocal<'a>(&'a AppState);

impl Local for AppLocal<'_> {
    fn with<R>(&self, f: impl FnOnce(&mut Store, &BlobStore) -> R) -> Result<R, SyncError> {
        let mut session = self.0.session.lock().map_err(|_| SyncError::Locked)?;
        let (store, blobs) = session.attachments().map_err(|_| SyncError::Locked)?;
        Ok(f(store, blobs))
    }
}

/// Starts the background loop (once).
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    if state.sync.started.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let sync = &state.sync;
        loop {
            tokio::select! {
                () = sync.now.notified() => {}
                () = sync.changed.notified() => {
                    // Wait for a pause in editing (bounded).
                    let first = Instant::now();
                    while first.elapsed() < MAX_DELAY {
                        tokio::select! {
                            () = sync.changed.notified() => {}
                            () = sync.now.notified() => break,
                            () = tokio::time::sleep(SETTLE) => break,
                        }
                    }
                }
                () = tokio::time::sleep(EVERY) => {}
            }
            run(&app).await;
        }
    });
}

/// One sync round, if connected and unlocked. Returns whether everything
/// local is in the cloud afterwards.
pub async fn run<R: Runtime>(app: &AppHandle<R>) -> bool {
    let state = app.state::<AppState>();
    let _round = state.sync.round.lock().await;

    let Some(provider) = state.cloud.provider() else {
        set_status(app, &state, |s| {
            s.state = SyncState::Off;
            s.message = None;
        });
        return false;
    };
    let Ok(keys) = state
        .session()
        .and_then(|s| s.keys().map_err(|e| e.to_string()))
    else {
        set_status(app, &state, |s| s.state = SyncState::Off);
        return false;
    };

    set_status(app, &state, |s| s.state = SyncState::Syncing);
    if !state.sync.folders_checked.load(Ordering::SeqCst) {
        match crate::cloud::check_cloud_folder(&state, &provider).await {
            Ok(()) => state.sync.folders_checked.store(true, Ordering::SeqCst),
            // Tried again next round; the sync below reports the problem.
            Err(e) => log::warn!("checking the Drive folder failed: {e}"),
        }
    }
    let started = Instant::now();
    let engine = SyncEngine::new(provider, keys, SyncConfig::default());
    let result = engine.sync(&AppLocal(&state)).await;
    let ok = result.is_ok();
    match result {
        Ok(report) => {
            // The sync log: counts and timings only, never content.
            if report.downloaded + report.uploaded + report.cleaned > 0
                || !report.changed.is_empty()
            {
                log::info!(
                    "sync: {} down, {} up, {} changed here, {} conflicts, {} cleaned up in {:?}",
                    report.downloaded,
                    report.uploaded,
                    report.changed.len(),
                    report.conflicts,
                    report.cleaned,
                    started.elapsed()
                );
            }
            if !report.changed.is_empty() {
                notify_changed(app, &report);
            }
            set_status(app, &state, |s| {
                s.state = SyncState::Idle;
                s.last_synced_at = Some(now_ms());
                s.message = None;
            });
        }
        Err(e) => {
            log::warn!("sync failed after {:?}: {e}", started.elapsed());
            let (state_now, message) = match &e {
                e if e.is_transient() => (SyncState::Offline, None),
                SyncError::Provider(ProviderError::Auth) => (
                    SyncState::Error,
                    Some(
                        "Google no longer accepts Memoir's sign-in. Connect again in Settings."
                            .into(),
                    ),
                ),
                SyncError::Provider(ProviderError::Quota) => {
                    (SyncState::Error, Some("Your Google Drive is full.".into()))
                }
                SyncError::Locked => (SyncState::Off, None),
                other => (SyncState::Error, Some(other.to_string())),
            };
            set_status(app, &state, |s| {
                s.state = state_now;
                s.message = message;
            });
        }
    }
    ok && state.sync.status().pending == 0
}

/// Pushes what's pending before the app exits (see shutdown.rs).
pub async fn flush<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let pending = state
        .session()
        .ok()
        .and_then(|mut s| s.store().ok()?.pending_changes().ok())
        .unwrap_or(0);
    if pending == 0 || state.cloud.provider().is_none() {
        return;
    }
    // A round may already be running (and finish the job); one more round
    // then pushes whatever came in meanwhile.
    for _ in 0..2 {
        if run(app).await {
            return;
        }
    }
}

fn notify_changed<R: Runtime>(app: &AppHandle<R>, report: &SyncReport) {
    let mut ids: Vec<ObjectId> = report.changed.clone();
    ids.sort();
    ids.dedup();
    if let Err(e) = app.emit(CHANGED_EVENT, ids) {
        log::warn!("could not tell the UI about synced changes: {e}");
    }
}

fn set_status<R: Runtime>(app: &AppHandle<R>, state: &AppState, f: impl FnOnce(&mut SyncStatus)) {
    let pending = state
        .session()
        .ok()
        .and_then(|mut s| s.store().ok()?.pending_changes().ok())
        .unwrap_or(0);
    let status = {
        let mut status = state.sync.status.lock().expect("lock");
        f(&mut status);
        status.pending = pending;
        status.clone()
    };
    if let Err(e) = app.emit(STATUS_EVENT, status) {
        log::warn!("could not send the sync status: {e}");
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[tauri::command(async)]
pub fn sync_status(state: State<'_, AppState>) -> SyncStatus {
    let mut status = state.sync.status();
    status.pending = state
        .session()
        .ok()
        .and_then(|mut s| s.store().ok()?.pending_changes().ok())
        .unwrap_or(0);
    if state.cloud.provider().is_none() {
        status.state = SyncState::Off;
    }
    status
}

#[tauri::command(async)]
pub fn sync_now(state: State<'_, AppState>) {
    state.sync.now();
}

//! Quitting must not lose anything:
//! 1. the UI is asked to save edits still waiting for autosave (≤ 2 s),
//! 2. the window is hidden (so closing feels instant),
//! 3. pending changes are pushed to the cloud (≤ 15 s),
//! 4. the process exits.
//!
//! Anything not pushed in time is still durable in the local outbox and is
//! pushed on next launch.
//!
//! On Windows closing the window quits. On macOS it only hides the window
//! (Mac apps keep running until you quit them): the editor is saved and
//! changes are pushed in the background, and Quit (⌘Q, see menu.rs) runs
//! the sequence above.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, Runtime, Window};
use tokio::sync::watch;

use crate::commands::AppState;

/// Event the UI answers with the `editor_flushed` command, passing back the
/// request number it carries.
pub const FLUSH_EDITOR_EVENT: &str = "flush-editor";
const EDITOR_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
const SYNC_FLUSH_TIMEOUT: Duration = Duration::from_secs(15);

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
/// Counts how often the window was shown again (macOS), so a close still
/// saving doesn't hide a window reopened meanwhile.
static SHOWN: AtomicU64 = AtomicU64::new(0);

/// Requests to save the editor and the UI's answers, matched by number: an
/// answer that comes after its request timed out never counts for a later
/// request (whose edits it doesn't cover).
pub struct EditorFlush {
    last_request: AtomicU64,
    /// Highest request number answered.
    answered: watch::Sender<u64>,
}

impl Default for EditorFlush {
    fn default() -> Self {
        Self {
            last_request: AtomicU64::new(0),
            answered: watch::Sender::new(0),
        }
    }
}

impl EditorFlush {
    /// The UI saved everything up to request `id`.
    pub fn answered(&self, id: u64) {
        self.answered.send_if_modified(|answered| {
            let newer = id > *answered;
            if newer {
                *answered = id;
            }
            newer
        });
    }
}

/// Quitting has started: nothing may bring the window back.
#[cfg(target_os = "macos")]
pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::SeqCst)
}

/// Call after showing the window again.
#[cfg(target_os = "macos")]
pub fn window_shown() {
    SHOWN.fetch_add(1, Ordering::SeqCst);
}

/// Call from `WindowEvent::CloseRequested` after `api.prevent_close()`.
pub fn on_close_requested<R: Runtime>(window: &Window<R>) {
    if cfg!(target_os = "macos") {
        hide(window);
    } else {
        quit(window);
    }
}

/// Saves, hides the window, pushes pending changes and exits.
pub fn quit<R: Runtime>(window: &Window<R>) {
    if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let window = window.clone();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();

        let state = app.state::<AppState>();
        save_editor(&app).await;
        if let Err(e) = window.hide() {
            log::warn!("could not hide window on close: {e}");
        }

        log::info!("shutdown: flushing pending changes");
        match tokio::time::timeout(SYNC_FLUSH_TIMEOUT, crate::sync::flush(&app)).await {
            Ok(()) => log::info!("shutdown: done in {:?}", started.elapsed()),
            Err(_) => log::warn!(
                "shutdown: sync timed out after {SYNC_FLUSH_TIMEOUT:?}; remaining changes stay queued"
            ),
        }
        // Decrypted copies of attachments opened in other programs.
        if let Ok(session) = state.session.lock() {
            session.clear_opened_files();
        }
        app.exit(0);
    });
}

/// Saves and hides the window; the app keeps running (macOS).
fn hide<R: Runtime>(window: &Window<R>) {
    let window = window.clone();
    let shown = SHOWN.load(Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        let app = window.app_handle();
        save_editor(app).await;
        // Reopened while saving (Dock icon, a menu command): leave it shown.
        if SHOWN.load(Ordering::SeqCst) == shown
            && let Err(e) = window.hide()
        {
            log::warn!("could not hide window on close: {e}");
        }
        app.state::<AppState>().sync.now();
    });
}

/// Asks the UI to save edits still waiting for autosave, and waits for it.
async fn save_editor<R: Runtime>(app: &AppHandle<R>) {
    let flush = &app.state::<AppState>().editor_flushed;
    let id = flush.last_request.fetch_add(1, Ordering::SeqCst) + 1;
    let mut answered = flush.answered.subscribe();
    match app.emit(FLUSH_EDITOR_EVENT, id) {
        Ok(()) => {
            if tokio::time::timeout(EDITOR_FLUSH_TIMEOUT, answered.wait_for(|&a| a >= id))
                .await
                .is_err()
            {
                log::warn!("shutdown: editor did not confirm saving in time");
            }
        }
        Err(e) => log::warn!("shutdown: could not ask the editor to save: {e}"),
    }
}

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

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, Runtime, Window};

use crate::commands::AppState;

/// Event the UI answers with the `editor_flushed` command.
pub const FLUSH_EDITOR_EVENT: &str = "flush-editor";
const EDITOR_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
const SYNC_FLUSH_TIMEOUT: Duration = Duration::from_secs(15);

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

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
    tauri::async_runtime::spawn(async move {
        let app = window.app_handle();
        save_editor(app).await;
        if let Err(e) = window.hide() {
            log::warn!("could not hide window on close: {e}");
        }
        app.state::<AppState>().sync.now();
    });
}

/// Asks the UI to save edits still waiting for autosave, and waits for it.
async fn save_editor<R: Runtime>(app: &AppHandle<R>) {
    match app.emit(FLUSH_EDITOR_EVENT, ()) {
        Ok(()) => {
            let state = app.state::<AppState>();
            if tokio::time::timeout(EDITOR_FLUSH_TIMEOUT, state.editor_flushed.notified())
                .await
                .is_err()
            {
                log::warn!("shutdown: editor did not confirm saving in time");
            }
        }
        Err(e) => log::warn!("shutdown: could not ask the editor to save: {e}"),
    }
}

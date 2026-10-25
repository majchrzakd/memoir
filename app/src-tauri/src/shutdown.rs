//! Closing the window must not lose anything:
//! 1. the UI is asked to save edits still waiting for autosave (≤ 2 s),
//! 2. the window is hidden (so closing feels instant),
//! 3. pending changes are pushed to the cloud (≤ 15 s),
//! 4. the process exits.
//!
//! Anything not pushed in time is still durable in the local outbox and is
//! pushed on next launch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager, Runtime, Window};

use crate::commands::AppState;

/// Event the UI answers with the `editor_flushed` command.
pub const FLUSH_EDITOR_EVENT: &str = "flush-editor";
const EDITOR_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
const SYNC_FLUSH_TIMEOUT: Duration = Duration::from_secs(15);

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Call from `WindowEvent::CloseRequested` after `api.prevent_close()`.
pub fn on_close_requested<R: Runtime>(window: &Window<R>) {
    if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let window = window.clone();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();

        let state = app.state::<AppState>();
        match app.emit(FLUSH_EDITOR_EVENT, ()) {
            Ok(()) => {
                if tokio::time::timeout(EDITOR_FLUSH_TIMEOUT, state.editor_flushed.notified())
                    .await
                    .is_err()
                {
                    log::warn!("shutdown: editor did not confirm saving in time");
                }
            }
            Err(e) => log::warn!("shutdown: could not ask the editor to save: {e}"),
        }
        if let Err(e) = window.hide() {
            log::warn!("could not hide window on close: {e}");
        }

        log::info!("shutdown: flushing pending changes");
        match tokio::time::timeout(SYNC_FLUSH_TIMEOUT, flush_outbox()).await {
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

/// Pushes the sync outbox. The sync engine arrives in M6; until then there
/// is nothing to push.
async fn flush_outbox() {}

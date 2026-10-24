//! Closing the window must not lose unsynced changes: the window is hidden
//! immediately (so closing feels instant), pending changes are flushed to the
//! cloud with a time limit, and only then does the process exit.
//!
//! Anything not flushed in time is still durable in the local outbox and is
//! pushed on next launch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{Manager, Runtime, Window};

const FLUSH_TIMEOUT: Duration = Duration::from_secs(15);

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Call from `WindowEvent::CloseRequested` after `api.prevent_close()`.
pub fn on_close_requested<R: Runtime>(window: &Window<R>) {
    if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(e) = window.hide() {
        log::warn!("could not hide window on close: {e}");
    }

    let app = window.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        log::info!("shutdown: flushing pending changes");
        match tokio::time::timeout(FLUSH_TIMEOUT, flush_outbox()).await {
            Ok(()) => log::info!("shutdown: flush finished in {:?}", started.elapsed()),
            Err(_) => log::warn!(
                "shutdown: flush timed out after {FLUSH_TIMEOUT:?}; remaining changes stay queued"
            ),
        }
        app.exit(0);
    });
}

/// Pushes the sync outbox. The sync engine arrives in M6; until then this
/// simulates network work so the close path can be exercised end to end.
async fn flush_outbox() {
    tokio::time::sleep(Duration::from_millis(1500)).await;
}

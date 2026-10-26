//! Windows shutdown or sign-out while changes aren't in the cloud yet:
//! Windows asks every window first (`WM_QUERYENDSESSION`). Memoir then
//! registers a reason Windows shows on its shutdown screen, runs the usual
//! closing sequence (save the editor, push changes, exit — see
//! shutdown.rs) and lets shutdown continue by exiting. With nothing
//! pending, it doesn't hold anything up.
//!
//! Hooking a window's messages needs the Win32 API directly, hence the
//! `unsafe` (allowed in this module only).

#![allow(unsafe_code)]

use std::sync::OnceLock;

use tauri::{Manager, WebviewWindow, Window};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Shutdown::ShutdownBlockReasonCreate;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::WM_QUERYENDSESSION;
use windows::core::PCWSTR;

use crate::commands::AppState;

const SUBCLASS_ID: usize = 0x4d45_4d4f; // "MEMO"
const REASON: &str = "Uploading your latest changes to Google Drive…";

static WINDOW: OnceLock<Window> = OnceLock::new();

/// Starts watching for shutdown on the main window. Call once, from setup.
pub fn install(window: &WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else {
        log::warn!("shutdown: no window handle; Windows shutdown won't wait for sync");
        return;
    };
    if WINDOW.set(window.as_ref().window()).is_err() {
        return;
    }
    // SAFETY: called on the thread that owns the window (setup runs on the
    // main thread); the callback is a plain function with no captured state.
    let ok = unsafe { SetWindowSubclass(hwnd, Some(on_message), SUBCLASS_ID, 0) };
    if !ok.as_bool() {
        log::warn!("shutdown: could not watch for Windows shutdown");
    }
}

/// Changes waiting to be pushed (and somewhere to push them).
fn unsynced() -> bool {
    let Some(window) = WINDOW.get() else {
        return false;
    };
    let state = window.state::<AppState>();
    if state.cloud.provider().is_none() {
        return false;
    }
    state
        .session
        .try_lock()
        .ok()
        .and_then(|mut s| s.store().ok()?.pending_changes().ok())
        // Busy (a save or sync holds the lock): assume there's something.
        .is_none_or(|n| n > 0)
}

unsafe extern "system" fn on_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_QUERYENDSESSION
        && unsynced()
        && let Some(window) = WINDOW.get()
    {
        log::info!("shutdown: Windows is shutting down with unsynced changes; pushing them first");
        let reason: Vec<u16> = REASON.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `reason` is a NUL-terminated UTF-16 string that outlives the call.
        if let Err(e) = unsafe { ShutdownBlockReasonCreate(hwnd, PCWSTR(reason.as_ptr())) } {
            log::warn!("shutdown: could not tell Windows why it should wait: {e}");
        }
        // Saves, pushes, then exits, which lets shutdown continue.
        crate::shutdown::on_close_requested(window);
        return LRESULT(0); // not yet
    }
    // SAFETY: forwards the message unchanged to the next handler.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

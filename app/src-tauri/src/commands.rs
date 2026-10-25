//! Commands the UI calls. Thin wrappers: the logic lives in [`Session`] and
//! `memoir_core::store`. All run off the main thread (`async`), so a slow
//! one (passphrase hashing) never freezes the window.

use std::sync::{Mutex, MutexGuard};

use memoir_core::store::{
    LaterItem, Note, ObjectId, Placement, Reply, SearchHit, Store, StoreError, TreeItem,
};
use tauri::State;
use tokio::sync::Notify;

use crate::session::{Session, Status};

pub struct AppState {
    pub session: Mutex<Session>,
    /// Signalled by the UI when it has saved pending edits (see shutdown.rs).
    pub editor_flushed: Notify,
}

type CmdResult<T> = Result<T, String>;

impl AppState {
    fn session(&self) -> CmdResult<MutexGuard<'_, Session>> {
        self.session
            .lock()
            .map_err(|_| "internal error: session lock poisoned".to_string())
    }

    fn with_store<T>(&self, f: impl FnOnce(&mut Store) -> Result<T, StoreError>) -> CmdResult<T> {
        let mut session = self.session()?;
        let store = session.store().map_err(|e| e.to_string())?;
        f(store).map_err(|e| e.to_string())
    }
}

// ---- Session ----------------------------------------------------------

#[tauri::command(async)]
pub fn status(state: State<'_, AppState>) -> CmdResult<Status> {
    Ok(state.session()?.status())
}

/// Returns the recovery key to show; nothing is saved until `finish_setup`.
#[tauri::command(async)]
pub fn begin_setup(state: State<'_, AppState>, passphrase: String) -> CmdResult<String> {
    let text = state
        .session()?
        .begin_setup(&passphrase)
        .map_err(|e| e.to_string())?;
    Ok(text.to_string())
}

#[tauri::command(async)]
pub fn finish_setup(state: State<'_, AppState>) -> CmdResult<()> {
    state.session()?.finish_setup().map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn unlock(state: State<'_, AppState>, passphrase: String, remember: bool) -> CmdResult<()> {
    state
        .session()?
        .unlock(&passphrase, remember)
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn unlock_with_recovery_key(
    state: State<'_, AppState>,
    recovery_key: String,
    new_passphrase: String,
    remember: bool,
) -> CmdResult<()> {
    state
        .session()?
        .unlock_with_recovery_key(&recovery_key, &new_passphrase, remember)
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn editor_flushed(state: State<'_, AppState>) {
    state.editor_flushed.notify_one();
}

// ---- Tree -------------------------------------------------------------

#[tauri::command(async)]
pub fn tree(state: State<'_, AppState>) -> CmdResult<Vec<TreeItem>> {
    state.with_store(|s| s.tree())
}

#[tauri::command(async)]
pub fn trashed_items(state: State<'_, AppState>) -> CmdResult<Vec<TreeItem>> {
    state.with_store(|s| s.trashed_items())
}

#[tauri::command(async)]
pub fn create_folder(
    state: State<'_, AppState>,
    placement: Placement,
    name: String,
) -> CmdResult<ObjectId> {
    state.with_store(|s| s.create_folder(placement, &name))
}

#[tauri::command(async)]
pub fn create_note(
    state: State<'_, AppState>,
    placement: Placement,
    title: String,
) -> CmdResult<ObjectId> {
    state.with_store(|s| s.create_note(placement, &title, ""))
}

#[tauri::command(async)]
pub fn rename(state: State<'_, AppState>, id: ObjectId, title: String) -> CmdResult<()> {
    state.with_store(|s| s.rename(&id, &title))
}

#[tauri::command(async)]
pub fn move_item(state: State<'_, AppState>, id: ObjectId, placement: Placement) -> CmdResult<()> {
    state.with_store(|s| s.move_item(&id, placement))
}

#[tauri::command(async)]
pub fn move_to_trash(state: State<'_, AppState>, id: ObjectId) -> CmdResult<()> {
    state.with_store(|s| s.move_to_trash(&id))
}

#[tauri::command(async)]
pub fn restore(state: State<'_, AppState>, id: ObjectId) -> CmdResult<()> {
    state.with_store(|s| s.restore(&id))
}

#[tauri::command(async)]
pub fn purge(state: State<'_, AppState>, id: ObjectId) -> CmdResult<()> {
    state.with_store(|s| s.purge(&id))
}

#[tauri::command(async)]
pub fn empty_trash(state: State<'_, AppState>) -> CmdResult<usize> {
    state.with_store(|s| s.empty_trash())
}

// ---- Notes ------------------------------------------------------------

#[tauri::command(async)]
pub fn note(state: State<'_, AppState>, id: ObjectId) -> CmdResult<Note> {
    state.with_store(|s| s.note(&id))
}

#[tauri::command(async)]
pub fn save_note(
    state: State<'_, AppState>,
    id: ObjectId,
    title: String,
    body: String,
) -> CmdResult<()> {
    state.with_store(|s| s.save_note(&id, &title, &body))
}

#[tauri::command(async)]
pub fn search(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
) -> CmdResult<Vec<SearchHit>> {
    state.with_store(|s| s.search(&query, limit.min(200)))
}

#[tauri::command(async)]
pub fn set_later(
    state: State<'_, AppState>,
    id: ObjectId,
    later: bool,
    due_date: Option<String>,
) -> CmdResult<()> {
    state.with_store(|s| s.set_later(&id, later, due_date.as_deref()))
}

#[tauri::command(async)]
pub fn later_notes(state: State<'_, AppState>) -> CmdResult<Vec<LaterItem>> {
    state.with_store(|s| s.later_notes())
}

// ---- Threads ----------------------------------------------------------

#[tauri::command(async)]
pub fn replies(state: State<'_, AppState>, note_id: ObjectId) -> CmdResult<Vec<Reply>> {
    state.with_store(|s| s.replies(&note_id))
}

#[tauri::command(async)]
pub fn add_reply(
    state: State<'_, AppState>,
    note_id: ObjectId,
    body: String,
) -> CmdResult<ObjectId> {
    state.with_store(|s| s.add_reply(&note_id, &body))
}

#[tauri::command(async)]
pub fn save_reply(state: State<'_, AppState>, id: ObjectId, body: String) -> CmdResult<()> {
    state.with_store(|s| s.save_note(&id, "", &body))
}

#[tauri::command(async)]
pub fn delete_reply(state: State<'_, AppState>, id: ObjectId) -> CmdResult<()> {
    state.with_store(|s| s.delete_reply(&id))
}

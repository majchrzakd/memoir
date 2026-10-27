//! "Export to Markdown": the notebook as plain files in a folder the user
//! picks (see `memoir_core::export`).

use std::path::{Path, PathBuf};

use memoir_core::export::{ExportSummary, export_markdown};
use serde::Serialize;
use tauri::{AppHandle, Runtime, State};
#[cfg(desktop)]
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::attachments::safe_file_name;
use crate::commands::AppState;

#[derive(Debug, Serialize)]
pub struct Exported {
    pub path: String,
    pub summary: ExportSummary,
}

/// Asks for a folder, then writes the export into a new folder `name` in
/// it. None if the user cancelled.
#[tauri::command(async)]
pub fn export_notebook<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<Exported>, String> {
    let Some(parent) = pick_folder(&app)? else {
        return Ok(None);
    };
    let out = new_folder(&parent, &safe_file_name(&name));
    let summary = {
        let mut session = state.session()?;
        let (store, blobs) = session.attachments().map_err(|e| e.to_string())?;
        export_markdown(store, blobs, &out).map_err(|e| e.to_string())?
    };
    Ok(Some(Exported {
        path: out.display().to_string(),
        summary,
    }))
}

#[cfg(desktop)]
fn pick_folder<R: Runtime>(app: &AppHandle<R>) -> Result<Option<PathBuf>, String> {
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Choose where to save the export")
        .blocking_pick_folder()
    else {
        return Ok(None);
    };
    picked.into_path().map(Some).map_err(|e| e.to_string())
}

/// iOS has no folder picker; exporting there will go through the Files app
/// (M10).
#[cfg(mobile)]
fn pick_folder<R: Runtime>(_app: &AppHandle<R>) -> Result<Option<PathBuf>, String> {
    Err("Export isn't available on iPhone yet.".into())
}

/// Opens the file manager at an export.
#[tauri::command(async)]
pub fn show_export<R: Runtime>(app: AppHandle<R>, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| e.to_string())
}

/// `parent/name`, or `parent/name (2)` etc. if that exists.
fn new_folder(parent: &Path, name: &str) -> PathBuf {
    let mut candidate = parent.join(name);
    let mut n = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{name} ({n})"));
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_folders_never_reuse_an_existing_one() {
        let dir = tempfile::tempdir().unwrap();
        let first = new_folder(dir.path(), "Memoir export");
        assert_eq!(first, dir.path().join("Memoir export"));
        std::fs::create_dir(&first).unwrap();
        assert_eq!(
            new_folder(dir.path(), "Memoir export"),
            dir.path().join("Memoir export (2)")
        );
    }
}

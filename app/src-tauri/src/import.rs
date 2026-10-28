//! "Import from Evernote": `.enex` files the user picks become folders of
//! notes (see `memoir_core::import`). The notebook is locked only one note
//! at a time, so the app stays usable and sync keeps going during a long
//! import.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use memoir_core::import::{ImportSummary, enex, import_enex_note};
use memoir_core::store::Placement;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::AppState;

pub const PROGRESS_EVENT: &str = "import-progress";

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    /// The file being imported (its name, which is the notebook's).
    pub file: String,
    /// Notes imported so far, over all files.
    pub notes: usize,
}

#[derive(Debug, Serialize)]
pub struct Imported {
    /// The folders created, one per file.
    pub folders: Vec<String>,
    pub summary: ImportSummary,
}

/// Asks for `.enex` files, then imports each into a new folder named after
/// it. None if the user cancelled.
#[tauri::command(async)]
pub fn import_evernote<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<Option<Imported>, String> {
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Choose Evernote export files")
        .add_filter("Evernote export", &["enex"])
        .blocking_pick_files()
    else {
        return Ok(None);
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);

    let mut imported = Imported {
        folders: Vec::new(),
        summary: ImportSummary::default(),
    };
    for path in picked {
        let path = path.into_path().map_err(|e| e.to_string())?;
        let name = notebook_name(&path);
        let result = import_file(&app, &state, &path, &name, now, &mut imported.summary);
        // What was imported stays, even if a later note or file fails.
        state.sync.changed();
        result.map_err(|e| {
            format!(
                "{e} (in {name}; {} notes were imported before this)",
                imported.summary.notes
            )
        })?;
        imported.folders.push(name);
    }
    log::info!(
        "import: {} notes, {} attachments from {} files",
        imported.summary.notes,
        imported.summary.attachments,
        imported.folders.len()
    );
    Ok(Some(imported))
}

fn import_file<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    path: &Path,
    name: &str,
    now: i64,
    total: &mut ImportSummary,
) -> Result<(), String> {
    let file = File::open(path).map_err(|e| format!("can't open {}: {e}", path.display()))?;
    let folder = state
        .session()?
        .store()
        .map_err(|e| e.to_string())?
        .create_folder(Placement::last_in(None), name)
        .map_err(|e| e.to_string())?;
    enex::read_notes(BufReader::new(file), |note| {
        let one = {
            let mut session = state
                .session()
                .map_err(memoir_core::import::ImportError::Format)?;
            let (store, blobs) = session
                .attachments()
                .map_err(|e| memoir_core::import::ImportError::Format(e.to_string()))?;
            import_enex_note(store, blobs, folder, &note, now)?
        };
        total.add(&one);
        let _ = app.emit(
            PROGRESS_EVENT,
            Progress {
                file: name.to_owned(),
                notes: total.notes,
            },
        );
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// Evernote names an exported notebook's file after it.
fn notebook_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().trim().to_owned())
        .unwrap_or_default();
    if stem.is_empty() {
        "Evernote".into()
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_named_after_the_files() {
        assert_eq!(
            notebook_name(Path::new("C:/x/Travel plans.enex")),
            "Travel plans"
        );
        assert_eq!(notebook_name(Path::new("/x/.enex")), ".enex");
        assert_eq!(notebook_name(Path::new("")), "Evernote");
    }
}

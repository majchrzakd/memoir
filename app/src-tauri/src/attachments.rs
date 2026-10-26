//! Attachments: adding them (raw bytes from the UI), showing images through
//! the `attachment:` URI scheme, and opening or saving files.
//!
//! Images are decrypted in memory for each request and sent with
//! `Cache-Control: no-store`, so no plaintext copy lands in the webview's
//! disk cache. Opening a file in another program needs a decrypted copy on
//! disk; those live in the session's "opened" folder, emptied on every start
//! and exit.

use std::fs;
use std::path::PathBuf;

use memoir_core::blobs::is_valid_id;
use serde::Serialize;
use tauri::http::{Response, StatusCode, header};
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::commands::AppState;

/// URI scheme the UI loads images from (`http://attachment.localhost/<id>` on Windows).
pub const SCHEME: &str = "attachment";
const MIME_HEADER: &str = "x-mime";

#[derive(Serialize)]
pub struct AttachmentInfo {
    pub id: String,
    pub size: u64,
    pub mime: String,
}

/// Body: the file's bytes; header `x-mime`: its type.
#[tauri::command(async)]
pub fn add_attachment(
    state: State<'_, AppState>,
    request: Request<'_>,
) -> Result<AttachmentInfo, String> {
    let InvokeBody::Raw(data) = request.body() else {
        return Err("expected the file's bytes".into());
    };
    let mime = request
        .headers()
        .get(MIME_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|m| is_plausible_mime(m))
        .unwrap_or("application/octet-stream")
        .to_ascii_lowercase();

    let mut session = state.session().map_err(|e| e.to_string())?;
    let (store, blobs) = session.attachments().map_err(|e| e.to_string())?;
    let id = blobs.put(data).map_err(|e| e.to_string())?;
    store
        .record_blob(&id, data.len() as u64, &mime)
        .map_err(|e| e.to_string())?;
    Ok(AttachmentInfo {
        id,
        size: data.len() as u64,
        mime,
    })
}

/// Opens an attachment in the program Windows associates with its type.
#[tauri::command(async)]
pub fn open_attachment<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<(), String> {
    let (data, dir) = {
        let mut session = state.session().map_err(|e| e.to_string())?;
        let dir = session.opened_dir().join(&id);
        let (_, blobs) = session.attachments().map_err(|e| e.to_string())?;
        (blobs.read(&id).map_err(|e| e.to_string())?, dir)
    };
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(safe_file_name(&name));
    fs::write(&path, data).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Asks where to save, then writes the decrypted attachment there.
#[tauri::command(async)]
pub fn save_attachment<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<bool, String> {
    let Some(target) = app
        .dialog()
        .file()
        .set_file_name(safe_file_name(&name))
        .blocking_save_file()
    else {
        return Ok(false); // cancelled
    };
    let path: PathBuf = target.into_path().map_err(|e| e.to_string())?;
    let data = {
        let mut session = state.session().map_err(|e| e.to_string())?;
        let (_, blobs) = session.attachments().map_err(|e| e.to_string())?;
        blobs.read(&id).map_err(|e| e.to_string())?
    };
    fs::write(path, data).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Answers `attachment:` requests: decrypted image bytes, never cached.
pub fn serve<R: Runtime>(
    app: &AppHandle<R>,
    request: &tauri::http::Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let id = request.uri().path().trim_start_matches('/');
    let empty = |status| {
        Response::builder()
            .status(status)
            .body(Vec::new())
            .expect("valid response")
    };
    if !is_valid_id(id) {
        return empty(StatusCode::BAD_REQUEST);
    }
    let state = app.state::<AppState>();
    let Ok(mut session) = state.session() else {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let Ok((store, blobs)) = session.attachments() else {
        return empty(StatusCode::FORBIDDEN); // locked
    };
    // Only images are displayed in notes; other files are opened or saved.
    let mime = match store.blob(id) {
        Ok(Some(meta)) if meta.mime.starts_with("image/") => meta.mime,
        Ok(_) => return empty(StatusCode::NOT_FOUND),
        Err(_) => return empty(StatusCode::INTERNAL_SERVER_ERROR),
    };
    match blobs.read(id) {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, mime)
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .body(data)
            .expect("valid response"),
        Err(e) => {
            log::warn!("could not read attachment {id}: {e}");
            empty(StatusCode::NOT_FOUND)
        }
    }
}

/// `type/subtype` made of safe characters (it ends up in a response header).
fn is_plausible_mime(mime: &str) -> bool {
    let mut parts = mime.split('/');
    let ok = |s: Option<&str>| {
        s.is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 100
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// A file name Windows accepts, with no directory parts.
pub(crate) fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_owned();
    if cleaned.is_empty() {
        "attachment".into()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(safe_file_name("report.pdf"), "report.pdf");
        assert_eq!(safe_file_name("..\\..\\evil.exe"), "evil.exe");
        assert_eq!(safe_file_name("a/b/c:d?.txt"), "c_d_.txt");
        assert_eq!(safe_file_name("   "), "attachment");
        assert_eq!(safe_file_name("name."), "name");
    }

    #[test]
    fn mime_types_are_checked() {
        assert!(is_plausible_mime("image/png"));
        assert!(is_plausible_mime("application/vnd.ms-excel"));
        assert!(is_plausible_mime("image/svg+xml"));
        for bad in [
            "",
            "image",
            "image/",
            "a/b/c",
            "text/html\r\nX: y",
            "image/png; charset=x",
        ] {
            assert!(!is_plausible_mime(bad), "{bad:?}");
        }
    }
}

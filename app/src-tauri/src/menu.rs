//! The macOS menu bar. Mac apps are expected to have one, and the standard
//! Edit menu is what makes ⌘C/⌘V/⌘Z reach the editor.
//!
//! Quit is our own item, not the standard one: the standard item ends the
//! process at once, skipping the closing sequence (shutdown.rs). Logging
//! out or quitting from the Dock still ends it at once; changes not pushed
//! by then stay in the local outbox and go up on next launch.
//!
//! Menu commands for the UI (new note, search, settings…) are sent as the
//! `menu-command` event with the item's id.

use tauri::menu::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub const MENU_COMMAND_EVENT: &str = "menu-command";

const QUIT: &str = "quit";
/// Items handled by the UI: (id, label, shortcut).
const SETTINGS: (&str, &str, &str) = ("settings", "Settings…", "Cmd+,");
const FILE_ITEMS: [(&str, &str, &str); 3] = [
    ("new-note", "New Note", "Cmd+N"),
    ("new-folder", "New Folder", "Shift+Cmd+N"),
    ("search", "Search…", "Cmd+K"),
];

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let ui_item = |(id, label, shortcut): (&str, &str, &str)| {
        MenuItem::with_id(app, id, label, true, Some(shortcut))
    };
    let separator = || PredefinedMenuItem::separator(app);
    let info = app.package_info();
    let about = AboutMetadata {
        name: Some(info.name.clone()),
        version: Some(info.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        ..Default::default()
    };

    let app_menu = Submenu::with_items(
        app,
        &info.name,
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &separator()?,
            &ui_item(SETTINGS)?,
            &separator()?,
            &PredefinedMenuItem::services(app, None)?,
            &separator()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &separator()?,
            &MenuItem::with_id(
                app,
                QUIT,
                format!("Quit {}", info.name),
                true,
                Some("Cmd+Q"),
            )?,
        ],
    )?;
    let [new_note, new_folder, search] = FILE_ITEMS;
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &ui_item(new_note)?,
            &ui_item(new_folder)?,
            &separator()?,
            &ui_item(search)?,
            &separator()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &separator()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &separator()?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &file, &edit, &window])
}

pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let id = event.id().as_ref();
    if id == QUIT {
        crate::shutdown::quit(&window.as_ref().window());
        return;
    }
    if id == SETTINGS.0 || FILE_ITEMS.iter().any(|(item, ..)| *item == id) {
        crate::show_window(app);
        if let Err(e) = app.emit(MENU_COMMAND_EVENT, id) {
            log::warn!("menu: could not pass {id} to the UI: {e}");
        }
    }
}

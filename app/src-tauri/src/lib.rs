mod commands;
mod session;
mod shutdown;

use memoir_core::keystore::OsKeychain;
use tauri::{Manager, WindowEvent};
use tokio::sync::Notify;

use crate::commands::AppState;
use crate::session::Session;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let dir = app.path().app_local_data_dir()?;
            let keychain = OsKeychain::new(app.config().identifier.clone());
            let session = Session::open(dir, Box::new(keychain));
            log::info!("session opened: {:?}", session.status());
            app.manage(AppState {
                session: std::sync::Mutex::new(session),
                editor_flushed: Notify::new(),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                shutdown::on_close_requested(window);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::begin_setup,
            commands::finish_setup,
            commands::unlock,
            commands::unlock_with_recovery_key,
            commands::editor_flushed,
            commands::tree,
            commands::trashed_items,
            commands::create_folder,
            commands::create_note,
            commands::rename,
            commands::move_item,
            commands::move_to_trash,
            commands::restore,
            commands::purge,
            commands::empty_trash,
            commands::note,
            commands::save_note,
            commands::search,
            commands::set_later,
            commands::later_notes,
            commands::replies,
            commands::add_reply,
            commands::save_reply,
            commands::delete_reply,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Memoir");
}

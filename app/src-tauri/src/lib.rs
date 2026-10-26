mod attachments;
mod cloud;
mod commands;
mod session;
mod shutdown;
mod sync;

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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol(
            attachments::SCHEME,
            |ctx, request, responder| {
                let app = ctx.app_handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    responder.respond(attachments::serve(&app, &request));
                });
            },
        )
        .setup(|app| {
            // Development builds can use a separate data folder (tests, demos)
            // so they never touch the real notebook.
            let dir = match std::env::var_os("MEMOIR_DATA_DIR") {
                Some(dir) if cfg!(debug_assertions) => std::path::PathBuf::from(dir),
                _ => app.path().app_local_data_dir()?,
            };
            let keychain = OsKeychain::new(app.config().identifier.clone());
            let session = Session::open(dir, Box::new(keychain));
            log::info!("session opened: {:?}", session.status());
            app.manage(AppState {
                session: std::sync::Mutex::new(session),
                cloud: cloud::Cloud::default(),
                sync: sync::Sync::default(),
                editor_flushed: Notify::new(),
            });
            cloud::restore(&app.state::<AppState>());
            sync::start(app.handle());
            app.state::<AppState>().sync.now();
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                shutdown::on_close_requested(window);
            }
            WindowEvent::Focused(true) => window.state::<AppState>().sync.focused(),
            _ => {}
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
            attachments::add_attachment,
            attachments::open_attachment,
            attachments::save_attachment,
            cloud::cloud_status,
            cloud::check_drive,
            cloud::connect_drive,
            cloud::cancel_connect,
            cloud::disconnect_drive,
            cloud::find_drive_notebook,
            cloud::open_drive_notebook,
            cloud::forget_drive_notebook,
            sync::sync_status,
            sync::sync_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Memoir");
}

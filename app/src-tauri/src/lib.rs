mod attachments;
mod cloud;
mod commands;
mod export;
mod logging;
mod session;
mod shutdown;
#[cfg(windows)]
mod shutdown_block;
mod sync;
mod update;

use memoir_core::keystore::OsKeychain;
use tauri::{Manager, WindowEvent};
use tokio::sync::Notify;

use crate::commands::AppState;
use crate::session::Session;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::log_panics();
    tauri::Builder::default()
        .plugin(logging::plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            log::info!(
                "Memoir {} starting ({} {})",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            let dir = match logging::data_dir_override() {
                Some(dir) => dir,
                None => app.path().app_local_data_dir()?,
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
            app.manage(update::Updates::default());
            cloud::restore(&app.state::<AppState>());
            sync::start(app.handle());
            update::start(app.handle());
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                shutdown_block::install(&window);
            }
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
            commands::note_history,
            commands::restore_version,
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
            export::export_notebook,
            export::show_export,
            logging::app_info,
            logging::show_logs,
            sync::sync_status,
            update::check_for_update,
            update::available_update,
            update::install_update,
            sync::sync_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Memoir");
}

mod attachments;
mod cloud;
mod commands;
mod export;
mod import;
mod logging;
#[cfg(target_os = "macos")]
mod menu;
mod session;
mod shutdown;
#[cfg(windows)]
mod shutdown_block;
mod sync;
mod update;
mod workspaces;

use memoir_core::keystore::OsKeychain;
#[cfg(target_os = "macos")]
use tauri::{AppHandle, RunEvent, Runtime};
use tauri::{Manager, WindowEvent};

use crate::commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::log_panics();
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(menu::on_event);
    builder
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
            let workspaces = workspaces::Workspaces::load(dir, std::sync::Arc::new(keychain));
            let session = workspaces.open_active();
            app.manage(workspaces);
            log::info!("session opened: {:?}", session.status());
            app.manage(AppState {
                session: std::sync::Mutex::new(session),
                cloud: cloud::Cloud::default(),
                sync: sync::Sync::default(),
                editor_flushed: Default::default(),
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
            commands::lock,
            commands::change_passphrase,
            commands::new_recovery_key,
            commands::tree,
            commands::trashed_items,
            commands::set_folder_password,
            commands::unlock_folder,
            commands::lock_folder,
            commands::forget_folder_password,
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
            commands::backlinks,
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
            import::import_evernote,
            export::show_export,
            logging::app_info,
            logging::show_logs,
            sync::sync_status,
            update::check_for_update,
            update::available_update,
            update::install_update,
            sync::sync_now,
            workspaces::workspaces,
            workspaces::switch_workspace,
            workspaces::add_workspace,
            workspaces::rename_workspace,
            workspaces::remove_workspace,
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Memoir")
        .run(|_app, _event| {
            // macOS: clicking the Dock icon brings back a closed (hidden) window.
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = _event {
                show_window(_app);
            }
            // iOS: Google's sign-in comes back through the app's URL scheme.
            #[cfg(target_os = "ios")]
            if let tauri::RunEvent::Opened { urls } = &_event {
                for url in urls {
                    _app.state::<AppState>().cloud.opened(url.as_str());
                }
            }
        });
}

/// Shows the main window again after it was closed (hidden) on macOS.
/// Not while quitting: edits made then would be lost when the app exits.
#[cfg(target_os = "macos")]
fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if shutdown::is_shutting_down() {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        if let Err(e) = window.show().and_then(|()| window.set_focus()) {
            log::warn!("could not show window: {e}");
        }
        shutdown::window_shown();
    }
}

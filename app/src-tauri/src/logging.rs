//! Logging: `logs/memoir.log` in the app's data folder, rotated (5 × 1 MB),
//! plus crashes (panics). Logs must never contain note content — titles,
//! text or file names — only ids, counts, timings and error kinds; keep it
//! that way when adding log lines.

use std::path::PathBuf;

use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};
use tauri_plugin_opener::OpenerExt;

const FILE_NAME: &str = "memoir";

/// Development builds can use a separate data folder (`MEMOIR_DATA_DIR`) so
/// they never touch the real notebook; their logs go there too.
pub fn data_dir_override() -> Option<PathBuf> {
    std::env::var_os("MEMOIR_DATA_DIR")
        .filter(|_| cfg!(debug_assertions))
        .map(PathBuf::from)
}

pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    let file = match data_dir_override() {
        Some(dir) => TargetKind::Folder {
            path: dir.join("logs"),
            file_name: Some(FILE_NAME.into()),
        },
        // %LOCALAPPDATA%\com.memoir.notebook\logs, next to the data.
        None => TargetKind::LogDir {
            file_name: Some(FILE_NAME.into()),
        },
    };
    let mut builder = tauri_plugin_log::Builder::new()
        .clear_targets()
        .target(Target::new(file))
        .max_file_size(1_000_000)
        .rotation_strategy(RotationStrategy::KeepSome(5))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .level(log::LevelFilter::Info)
        // Libraries: only problems.
        .level_for("tao", log::LevelFilter::Warn)
        .level_for("wry", log::LevelFilter::Warn)
        .level_for("reqwest", log::LevelFilter::Warn)
        .level_for("hyper", log::LevelFilter::Warn)
        .level_for("hyper_util", log::LevelFilter::Warn)
        .level_for("rustls", log::LevelFilter::Warn);
    if cfg!(debug_assertions) {
        builder = builder.target(Target::new(TargetKind::Stdout));
    }
    builder.build()
}

/// Records crashes in the log before the default handling.
pub fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("crash: {info}");
        previous(info);
    }));
}

fn log_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    match data_dir_override() {
        Some(dir) => Ok(dir.join("logs")),
        None => app.path().app_log_dir().map_err(|e| e.to_string()),
    }
}

#[derive(serde::Serialize)]
pub struct AppInfo {
    pub version: String,
    pub log_dir: String,
}

#[tauri::command(async)]
pub fn app_info<R: Runtime>(app: AppHandle<R>) -> Result<AppInfo, String> {
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        log_dir: log_dir(&app)?.display().to_string(),
    })
}

/// Opens the log folder (to attach logs to a bug report).
#[tauri::command(async)]
pub fn show_logs<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let dir = log_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

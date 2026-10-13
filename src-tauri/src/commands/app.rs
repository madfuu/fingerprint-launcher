//! Settings, app information and the window / exit commands.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::db::settings::{self, Settings, SettingsPatch};
use crate::error::AppResult;
use crate::shell;
use crate::state::AppState;

/// The OS the launcher runs on (the title bar differs on macOS, design §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostOs {
    Windows,
    Macos,
    Linux,
}

impl HostOs {
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

/// `getAppInfo`: facts the settings page and title bar need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub os: HostOs,
    pub data_dir: String,
    /// Where default user data directories go while the `userDataRoot` setting is unset.
    pub default_user_data_root: String,
    pub log_dir: String,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> AppResult<Settings> {
    state.db(settings::get).await
}

/// Applies the fields present in `patch`; returns the resulting settings.
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> AppResult<Settings> {
    state.db(move |conn| settings::update(conn, patch)).await
}

#[tauri::command]
pub async fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppResult<AppInfo> {
    let text = |path: std::path::PathBuf| path.to_string_lossy().into_owned();
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        os: HostOs::current(),
        data_dir: text(state.paths.data_dir().to_owned()),
        default_user_data_root: text(state.paths.default_user_data_root()),
        log_dir: text(state.paths.log_dir()),
    })
}

/// Called by the frontend once it has rendered: shows the window that was created hidden.
/// `boot_ok` is false when it rendered its error screen instead of the app (settings could not
/// be read, rendering threw, ...); `failure` then says why. See `shell::frontend_ready`.
#[tauri::command]
pub async fn show_main_window(
    app: AppHandle,
    boot_ok: bool,
    failure: Option<String>,
) -> AppResult<()> {
    shell::frontend_ready(&app, boot_ok, failure.as_deref());
    Ok(())
}

/// Stops every browser and exits (after the user confirmed `app://close-requested`). Calls
/// while quitting return at once.
#[tauri::command]
pub async fn quit_app(app: AppHandle) -> AppResult<()> {
    shell::quit(app).await;
    Ok(())
}

//! Command line preview and browser lifecycle commands.

use std::time::Duration;

use tauri::State;

use crate::db::profiles::ProfileDraft;
use crate::db::settings;
use crate::error::AppResult;
use crate::kernel::args::{ArgSupport, FieldWarning, LaunchPlan};
use crate::services::launcher::{BatchProgress, RunningInfo};
use crate::services::preview;
use crate::state::AppState;

/// The command line of an editor draft. `profile_id` is the id of the profile being edited
/// (`null` for a new one); it names the default user data directory.
#[tauri::command]
pub async fn preview_command(
    state: State<'_, AppState>,
    draft: ProfileDraft,
    profile_id: Option<String>,
) -> AppResult<LaunchPlan> {
    let default_root = state.paths.default_user_data_root();
    state
        .db(move |conn| {
            preview::preview_command(conn, &default_root, &draft, profile_id.as_deref())
        })
        .await
}

/// Arguments the kernel accepts (`null` = the default kernel).
#[tauri::command]
pub async fn get_arg_support(
    state: State<'_, AppState>,
    kernel_id: Option<String>,
) -> AppResult<ArgSupport> {
    state
        .db(move |conn| preview::arg_support(conn, kernel_id.as_deref()))
        .await
}

/// Set fields the draft's kernel (`draft.kernelId`, else the default) would not pass on.
#[tauri::command]
pub async fn validate_profile_for_kernel(
    state: State<'_, AppState>,
    draft: ProfileDraft,
) -> AppResult<Vec<FieldWarning>> {
    state
        .db(move |conn| preview::validate_for_kernel(conn, &draft))
        .await
}

/// Resolves once the browser process exists; later states arrive as status events.
#[tauri::command]
pub async fn launch_profile(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.launcher.launch(&id).await.map(drop)
}

/// Resolves once the browser has stopped (at most the stop grace period plus a kill).
#[tauri::command]
pub async fn stop_profile(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.launcher.stop(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn list_running(state: State<'_, AppState>) -> AppResult<Vec<RunningInfo>> {
    Ok(state.launcher.list_running())
}

/// Launches in order with the configured concurrency and interval (design §4.6). Progress
/// arrives as `batch://progress`; resolves with the final count when the batch is done.
#[tauri::command]
pub async fn batch_launch(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> AppResult<BatchProgress> {
    let settings = state.db(settings::get).await?;
    Ok(state
        .launcher
        .batch_launch(
            &ids,
            settings.batch_concurrency,
            Duration::from_millis(settings.batch_interval_ms.into()),
        )
        .await)
}

#[tauri::command]
pub async fn batch_stop(state: State<'_, AppState>, ids: Vec<String>) -> AppResult<()> {
    state.launcher.stop_many(&ids).await;
    Ok(())
}

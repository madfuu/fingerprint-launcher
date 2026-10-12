//! Profile commands.

use tauri::State;

use crate::db::Page;
use crate::db::profiles::{self, Profile, ProfileDraft, ProfileQuery, ProfileRow};
use crate::error::AppResult;
use crate::services::profiles as service;
use crate::services::profiles::PurgeResult;
use crate::services::transfer::{self, ImportResult};
use crate::state::AppState;

#[tauri::command]
pub async fn list_profiles(
    state: State<'_, AppState>,
    query: ProfileQuery,
) -> AppResult<Page<ProfileRow>> {
    service::list(&state, query).await
}

#[tauri::command]
pub async fn get_profile(state: State<'_, AppState>, id: String) -> AppResult<Profile> {
    state.db(move |conn| profiles::get(conn, &id)).await
}

#[tauri::command]
pub async fn create_profile(state: State<'_, AppState>, draft: ProfileDraft) -> AppResult<Profile> {
    service::create(&state, draft).await
}

#[tauri::command]
pub async fn update_profile(
    state: State<'_, AppState>,
    id: String,
    draft: ProfileDraft,
) -> AppResult<Profile> {
    service::update(&state, id, draft).await
}

/// Moves a profile to the trash (`profile_running` while it runs).
#[tauri::command]
pub async fn delete_profile(state: State<'_, AppState>, id: String) -> AppResult<()> {
    service::soft_delete(&state, id).await
}

#[tauri::command]
pub async fn restore_profile(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.db(move |conn| profiles::restore(conn, &id)).await
}

#[tauri::command]
pub async fn purge_profile(
    state: State<'_, AppState>,
    id: String,
    drop_user_data: bool,
) -> AppResult<PurgeResult> {
    service::purge(&state, id, drop_user_data).await
}

#[tauri::command]
pub async fn duplicate_profile(state: State<'_, AppState>, id: String) -> AppResult<Profile> {
    service::duplicate(&state, id).await
}

/// Writes the profiles to `path` (from the save dialog); returns how many.
#[tauri::command]
pub async fn export_profiles(
    state: State<'_, AppState>,
    ids: Vec<String>,
    path: String,
) -> AppResult<u32> {
    transfer::export_profiles(&state, ids, path).await
}

#[tauri::command]
pub async fn import_profiles(state: State<'_, AppState>, path: String) -> AppResult<ImportResult> {
    transfer::import_profiles(&state, path).await
}

#[tauri::command]
pub async fn set_profile_tags(
    state: State<'_, AppState>,
    profile_id: String,
    tag_ids: Vec<String>,
) -> AppResult<()> {
    state
        .db(move |conn| profiles::set_tags(conn, &profile_id, &tag_ids))
        .await
}

/// Moves profiles into a group, or out of every group with `null`.
#[tauri::command]
pub async fn batch_set_group(
    state: State<'_, AppState>,
    ids: Vec<String>,
    group_id: Option<String>,
) -> AppResult<()> {
    state
        .db(move |conn| profiles::set_group(conn, &ids, group_id.as_deref()))
        .await
}

#[tauri::command]
pub async fn batch_add_tags(
    state: State<'_, AppState>,
    ids: Vec<String>,
    tag_ids: Vec<String>,
) -> AppResult<()> {
    state
        .db(move |conn| profiles::add_tags(conn, &ids, &tag_ids))
        .await
}

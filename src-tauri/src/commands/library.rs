//! Proxy, group and tag commands: the shared lists profiles refer to.

use tauri::State;

use crate::db::groups::{self, Group, GroupInput};
use crate::db::proxies::{self, Proxy, ProxyInput};
use crate::db::tags::{self, Tag, TagInput};
use crate::error::AppResult;
use crate::services::proxy_check::{self, ProxyCheck};
use crate::state::AppState;

#[tauri::command]
pub async fn list_proxies(state: State<'_, AppState>) -> AppResult<Vec<Proxy>> {
    state.db(proxies::list).await
}

#[tauri::command]
pub async fn create_proxy(state: State<'_, AppState>, input: ProxyInput) -> AppResult<Proxy> {
    state.db(move |conn| proxies::create(conn, input)).await
}

#[tauri::command]
pub async fn update_proxy(
    state: State<'_, AppState>,
    id: String,
    input: ProxyInput,
) -> AppResult<Proxy> {
    state
        .db(move |conn| proxies::update(conn, &id, input))
        .await
}

/// Profiles using it go without a proxy.
#[tauri::command]
pub async fn delete_proxy(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.db(move |conn| proxies::delete(conn, &id)).await
}

/// Looks up the exit IP through the proxy (network access; only on user request).
#[tauri::command]
pub async fn test_proxy(state: State<'_, AppState>, id: String) -> AppResult<ProxyCheck> {
    let proxy = state.db(move |conn| proxies::get(conn, &id)).await?;
    proxy_check::check(&proxy).await
}

#[tauri::command]
pub async fn list_groups(state: State<'_, AppState>) -> AppResult<Vec<Group>> {
    state.db(groups::list).await
}

#[tauri::command]
pub async fn create_group(state: State<'_, AppState>, input: GroupInput) -> AppResult<Group> {
    state.db(move |conn| groups::create(conn, input)).await
}

#[tauri::command]
pub async fn update_group(
    state: State<'_, AppState>,
    id: String,
    input: GroupInput,
) -> AppResult<Group> {
    state.db(move |conn| groups::update(conn, &id, input)).await
}

/// Its profiles become ungrouped.
#[tauri::command]
pub async fn delete_group(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.db(move |conn| groups::delete(conn, &id)).await
}

#[tauri::command]
pub async fn list_tags(state: State<'_, AppState>) -> AppResult<Vec<Tag>> {
    state.db(tags::list).await
}

#[tauri::command]
pub async fn create_tag(state: State<'_, AppState>, input: TagInput) -> AppResult<Tag> {
    state.db(move |conn| tags::create(conn, input)).await
}

#[tauri::command]
pub async fn update_tag(state: State<'_, AppState>, id: String, input: TagInput) -> AppResult<Tag> {
    state.db(move |conn| tags::update(conn, &id, input)).await
}

/// Removes it from every profile.
#[tauri::command]
pub async fn delete_tag(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.db(move |conn| tags::delete(conn, &id)).await
}

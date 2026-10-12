//! Kernel commands.

use tauri::State;

use crate::db::kernels::{self, Kernel, KernelInput};
use crate::error::AppResult;
use crate::kernel::probe::KernelProbe;
use crate::services::kernels::{self as service, KernelCandidate};
use crate::state::AppState;

#[tauri::command]
pub async fn list_kernels(state: State<'_, AppState>) -> AppResult<Vec<Kernel>> {
    state.db(kernels::list).await
}

/// Registers a kernel. `major_version` overrides the probed version and is required
/// (`kernel_version_unknown`) when probing finds none.
#[tauri::command]
pub async fn add_kernel(
    state: State<'_, AppState>,
    path: String,
    name: Option<String>,
    major_version: Option<u32>,
) -> AppResult<Kernel> {
    service::add_kernel(&state, path, name, major_version).await
}

#[tauri::command]
pub async fn update_kernel(
    state: State<'_, AppState>,
    id: String,
    input: KernelInput,
) -> AppResult<Kernel> {
    service::update_kernel(&state, id, input).await
}

/// Profiles bound to it fall back to the default kernel; running browsers are not affected.
#[tauri::command]
pub async fn remove_kernel(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.db(move |conn| kernels::delete(conn, &id)).await
}

#[tauri::command]
pub async fn set_default_kernel(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.db(move |conn| kernels::set_default(conn, &id)).await
}

/// Candidates for the user to confirm; nothing is added.
#[tauri::command]
pub async fn detect_kernels(state: State<'_, AppState>) -> AppResult<Vec<KernelCandidate>> {
    service::detect_kernels(&state).await
}

/// The version of the kernel at `path` (absolute); all fields `null` when it cannot be found.
#[tauri::command]
pub async fn probe_kernel(path: String) -> AppResult<KernelProbe> {
    service::probe(&path).await
}

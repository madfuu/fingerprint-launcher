//! Fingerprint Launcher — Tauri application entry point.

pub mod commands;
pub mod db;
pub mod error;
pub mod events;
pub mod kernel;
pub mod paths;
pub mod services;
pub mod shell;
pub mod state;

#[cfg(test)]
mod bindings;

use std::sync::Arc;

use tauri::Manager;

use crate::events::TauriSink;
use crate::paths::AppPaths;
use crate::shell::ShellState;
use crate::state::AppState;

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(ShellState::from_env())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let sink = Arc::new(TauriSink::new(app.handle().clone()));
            let state = AppState::open(AppPaths::new(data_dir), sink)?;
            app.manage(state);
            shell::create_main_window(app.handle())?;
            Ok(())
        })
        .on_window_event(shell::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::profiles::list_profiles,
            commands::profiles::get_profile,
            commands::profiles::create_profile,
            commands::profiles::update_profile,
            commands::profiles::delete_profile,
            commands::profiles::restore_profile,
            commands::profiles::purge_profile,
            commands::profiles::duplicate_profile,
            commands::profiles::export_profiles,
            commands::profiles::import_profiles,
            commands::profiles::set_profile_tags,
            commands::profiles::batch_set_group,
            commands::profiles::batch_add_tags,
            commands::launch::preview_command,
            commands::launch::get_arg_support,
            commands::launch::validate_profile_for_kernel,
            commands::launch::launch_profile,
            commands::launch::stop_profile,
            commands::launch::list_running,
            commands::launch::batch_launch,
            commands::launch::batch_stop,
            commands::kernels::list_kernels,
            commands::kernels::add_kernel,
            commands::kernels::update_kernel,
            commands::kernels::remove_kernel,
            commands::kernels::set_default_kernel,
            commands::kernels::detect_kernels,
            commands::kernels::probe_kernel,
            commands::library::list_proxies,
            commands::library::create_proxy,
            commands::library::update_proxy,
            commands::library::delete_proxy,
            commands::library::test_proxy,
            commands::library::list_groups,
            commands::library::create_group,
            commands::library::update_group,
            commands::library::delete_group,
            commands::library::list_tags,
            commands::library::create_tag,
            commands::library::update_tag,
            commands::library::delete_tag,
            commands::app::get_settings,
            commands::app::update_settings,
            commands::app::get_app_info,
            commands::app::show_main_window,
            commands::app::quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    app.run(shell::on_run_event);
}

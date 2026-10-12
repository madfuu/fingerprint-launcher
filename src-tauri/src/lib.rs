//! Fingerprint Launcher — Tauri application entry point.

pub mod db;
pub mod error;
pub mod kernel;

use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

/// Label of the single application window. Referenced by `capabilities/default.json`.
pub const MAIN_WINDOW_LABEL: &str = "main";

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            create_main_window(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Creates the main window in code (`app.windows` in `tauri.conf.json` is empty) so the
/// title bar can differ per platform. Dimensions follow design §7.
fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let builder =
        WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, WebviewUrl::App("index.html".into()))
            .title(&app.package_info().name)
            .inner_size(1438.0, 828.0)
            .min_inner_size(1038.0, 688.0)
            .resizable(true)
            .maximizable(true)
            .center()
            // Native file drops are not used; this keeps HTML5 drag and drop working in the
            // WebView on every platform (`drag_and_drop(false)` exists on Windows only).
            .disable_drag_drop_handler()
            // Shown immediately for now. P5 switches to `visible(false)` and lets the frontend
            // call `showMainWindow` once settings, i18n and theme are loaded (no white flash).
            .visible(true);

    // macOS keeps the native traffic lights over a transparent title bar; elsewhere the
    // frontend draws its own title bar and window controls.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);

    builder.build()?;
    Ok(())
}

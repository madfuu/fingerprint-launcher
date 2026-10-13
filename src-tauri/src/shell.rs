//! The desktop shell around the services: the main window, showing it once the frontend is
//! ready, and quitting without leaving browsers behind (design §4.5 / §7).
//!
//! Exit paths, as Tauri 2.12 / tao 0.37 deliver them:
//!
//! - Closing the window (our title bar's close button, Alt+F4, the window manager) raises
//!   `CloseRequested` while the window still exists. With browsers running, the close is
//!   prevented and the frontend asked to confirm (`app://close-requested`); it then calls
//!   `quitApp`, which stops every browser and exits.
//! - `RunEvent::ExitRequested` with no exit code only follows the destruction of the last
//!   window, when nothing is left to show a prompt in; it is not intercepted (one prompt per
//!   close, never two).
//! - macOS Cmd+Q / Dock "Quit" terminate the application directly: tao reports neither a close
//!   nor an exit request, only `RunEvent::Exit`.
//!
//! Every path ends in `RunEvent::Exit`, which stops whatever is still running before the
//! process goes away (A7).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{
    AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
};

use crate::events::{APP_CLOSE_REQUESTED, CloseRequested};
use crate::state::AppState;

/// Label of the single application window. Referenced by `capabilities/default.json`.
pub const MAIN_WINDOW_LABEL: &str = "main";

/// The window is created hidden and shown by the frontend once settings, i18n and theme are
/// applied (no white flash). If the frontend has not done so by then, it is shown anyway: a
/// broken frontend must not leave an invisible app behind.
pub const SHOW_FALLBACK: Duration = Duration::from_secs(5);

/// Set to `1`, the app exits as soon as the frontend calls `showMainWindow`: with code 0 when
/// it reports a successful boot, 1 when it reports a failure (the reason goes to stderr). CI's
/// smoke test uses it to prove the WebView loaded the frontend, completed an IPC round trip
/// under the production CSP, and rendered the app rather than its error screen.
pub const SMOKE_EXIT_ON_READY_ENV: &str = "FPL_SMOKE_EXIT_ON_READY";

/// Exit code of a smoke run that ended without the frontend ever calling `showMainWindow`: the
/// event loop exits with code 0 once the last window is destroyed, which would otherwise read
/// as a passing smoke test (see `on_run_event`).
const SMOKE_NOT_READY_EXIT: i32 = 3;

/// Where the macOS traffic lights sit in our 48 px title bar: vertically centred, with the
/// brand slot starting after them (the frontend reserves 78 px on macOS).
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_POSITION: tauri::LogicalPosition<f64> = tauri::LogicalPosition::new(18.0, 18.0);

/// Window and exit flags.
#[derive(Debug, Default)]
pub struct ShellState {
    shown: AtomicBool,
    /// The frontend called `showMainWindow`, so it can answer `app://close-requested`.
    frontend_ready: AtomicBool,
    /// `quitApp` is stopping the browsers; the app exits when it is done.
    exiting: AtomicBool,
    smoke_exit_on_ready: bool,
}

impl ShellState {
    pub fn from_env() -> Self {
        Self {
            smoke_exit_on_ready: std::env::var(SMOKE_EXIT_ON_READY_ENV).as_deref() == Ok("1"),
            ..Self::default()
        }
    }
}

/// Creates the main window in code (`app.windows` in `tauri.conf.json` is empty) so the title
/// bar can differ per platform. Dimensions follow design §7.
pub fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
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
            .visible(false);

    // macOS keeps the native traffic lights over a transparent title bar; elsewhere the
    // frontend draws its own title bar and window controls.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(TRAFFIC_LIGHT_POSITION);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);

    builder.build()?;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SHOW_FALLBACK).await;
        show(&app);
    });
    Ok(())
}

/// `showMainWindow`: the frontend has rendered, either the app (`boot_ok`) or its error screen
/// (`failure` says why). The window is shown either way.
pub fn frontend_ready(app: &AppHandle, boot_ok: bool, failure: Option<&str>) {
    if !boot_ok {
        eprintln!(
            "frontend startup failed: {}",
            failure.unwrap_or("no reason given")
        );
    }
    let shell = app.state::<ShellState>();
    shell.frontend_ready.store(true, Ordering::SeqCst);
    show(app);
    if shell.smoke_exit_on_ready {
        shell.exiting.store(true, Ordering::SeqCst);
        app.exit(if boot_ok { 0 } else { 1 });
    }
}

/// Shows and focuses the main window, once.
fn show(app: &AppHandle) {
    if app.state::<ShellState>().shown.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        if let Err(err) = window.show() {
            eprintln!("cannot show the main window: {err}");
        }
        let _ = window.set_focus();
    }
}

/// `quitApp`: stops every browser, then exits. Calls while a quit is in progress return at
/// once.
pub async fn quit(app: AppHandle) {
    if app
        .state::<ShellState>()
        .exiting
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    if let Some(state) = app.try_state::<AppState>() {
        state.launcher.shutdown().await;
    }
    app.exit(0);
}

/// `Builder::on_window_event`.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    let app = window.app_handle();
    let shell = app.state::<ShellState>();
    if shell.exiting.load(Ordering::SeqCst) {
        // `quitApp` is stopping the browsers and exits when done.
        api.prevent_close();
        return;
    }
    let running = app
        .try_state::<AppState>()
        .map_or(0, |state| state.launcher.running_ids().len());
    if running == 0 {
        return;
    }
    api.prevent_close();
    if shell.frontend_ready.load(Ordering::SeqCst) {
        let payload = CloseRequested {
            running_count: u32::try_from(running).unwrap_or(u32::MAX),
        };
        if app.emit(APP_CLOSE_REQUESTED, payload).is_ok() {
            return;
        }
    }
    // No frontend to ask: quit as if confirmed rather than refuse to close forever.
    tauri::async_runtime::spawn(quit(app.clone()));
}

/// `App::run` callback.
pub fn on_run_event(app: &AppHandle, event: RunEvent) {
    if let RunEvent::Exit = event {
        // Last stop on every exit path. Normally nothing is left (quitApp stopped everything,
        // or nothing ran); after Cmd+Q on macOS this is where browsers are closed. Bounded by
        // the launcher's stop grace period.
        if let Some(state) = app.try_state::<AppState>() {
            let launcher = state.launcher.clone();
            tauri::async_runtime::block_on(launcher.shutdown());
        }
        let shell = app.state::<ShellState>();
        if shell.smoke_exit_on_ready && !shell.frontend_ready.load(Ordering::SeqCst) {
            // Destroying the last window exits the event loop with code 0 (a crashed WebView,
            // a close we did not intercept). A smoke run may only succeed through
            // `frontend_ready`, so leave a distinguishable code behind instead.
            eprintln!("the app exited before the frontend called showMainWindow");
            std::process::exit(SMOKE_NOT_READY_EXIT);
        }
    }
}

//! Events pushed to the frontend (design §5). Names are fixed strings the frontend listens on;
//! payload types are exported to TypeScript with the other bridge types.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::services::launcher::{BatchProgress, StatusEvent, StatusSink};

/// Payload [`StatusEvent`].
pub const PROFILE_STATUS_CHANGED: &str = "profile://status-changed";
/// Payload [`BatchProgress`].
pub const BATCH_PROGRESS: &str = "batch://progress";
/// Payload [`CloseRequested`].
pub const APP_CLOSE_REQUESTED: &str = "app://close-requested";

/// The user asked to close the window while browsers are running: the frontend confirms, then
/// calls `quitApp` (design §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CloseRequested {
    pub running_count: u32,
}

/// Forwards launcher events to the frontend.
///
/// The launcher calls this with its table locked, so it only emits: no database access (the
/// command layer takes the locks in the other order), and no Rust-side `listen` handlers that
/// call back into the launcher (Tauri runs those synchronously inside `emit`).
pub struct TauriSink {
    app: AppHandle,
}

impl TauriSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl StatusSink for TauriSink {
    fn status_changed(&self, event: StatusEvent) {
        // Fails only when the webview is gone (the app is exiting); nobody is left to tell.
        let _ = self.app.emit(PROFILE_STATUS_CHANGED, event);
    }

    fn batch_progress(&self, progress: BatchProgress) {
        let _ = self.app.emit(BATCH_PROGRESS, progress);
    }
}

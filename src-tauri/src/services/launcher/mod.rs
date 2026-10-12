//! Launching, watching and stopping profiles (design §4).
//!
//! [`Launcher`] owns the table of profiles it has started. Its rules:
//!
//! - After `Starting`, every status change of a browser happens in its waiter task
//!   (`waiter.rs`); stop requests reach the waiter through a channel.
//! - Every status change updates the table and is emitted to the [`StatusSink`] while the
//!   table lock is held, so events arrive in the order the table changed — a profile's
//!   `Stopped` can never overtake the `Starting` of its next launch.
//! - Nothing depends on Tauri: profiles come from a [`LaunchSource`], events go to a
//!   [`StatusSink`], and time limits come from [`LauncherConfig`].
//!
//! Running state is memory only (design decision 2); it is gone after a restart.

mod source;
mod stderr;
mod waiter;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot, watch};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::kernel::lock::{self, LockState};
use crate::kernel::process;

pub use source::{DbLaunchSource, LaunchRequest, LaunchSource, user_data_dir};
pub use stderr::STDERR_TAIL_LINES;

/// Time limits of the browser lifecycle. Tests shorten them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherConfig {
    /// A browser alive this long after its spawn is `Running`; one that exits sooner, with
    /// any code, failed to start.
    pub startup_grace: Duration,
    /// A browser that exits with a failure this soon after its spawn failed to start too.
    pub early_exit_window: Duration,
    /// How long a browser asked to close may take before its tree is killed.
    pub stop_grace: Duration,
    /// Directory of the per-profile stderr logs (`<profile id>.log`); `None` = no log files.
    pub log_dir: Option<PathBuf>,
}

impl LauncherConfig {
    pub const STARTUP_GRACE: Duration = Duration::from_secs(3);
    pub const EARLY_EXIT_WINDOW: Duration = Duration::from_secs(10);
    /// Conservative; to be tuned on real machines (research/process-lifecycle.md §3).
    pub const STOP_GRACE: Duration = Duration::from_secs(5);
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            startup_grace: Self::STARTUP_GRACE,
            early_exit_window: Self::EARLY_EXIT_WINDOW,
            stop_grace: Self::STOP_GRACE,
            log_dir: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    /// The launch failed; the profile is not running. See [`StatusEvent::error`].
    Error,
}

/// A known cause of a failed start, for an actionable message. The frontend picks the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureHint {
    /// `No usable sandbox`: add `--no-sandbox` to the profile's extra arguments.
    NoSandbox,
    /// The AppImage needs FUSE 2: install libfuse2 or use the tar.xz build.
    MissingFuse,
}

/// Why a launch failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchFailure {
    /// An [`ErrorCode`]: the pre-spawn codes, or `startup_failed` when the browser exited
    /// during startup.
    pub code: ErrorCode,
    /// Raw detail, including the last stderr lines.
    pub message: String,
    pub hint: Option<FailureHint>,
    pub exit_code: Option<i32>,
    /// The last [`STDERR_TAIL_LINES`] stderr lines, oldest first.
    pub stderr_tail: Vec<String>,
}

impl From<&AppError> for LaunchFailure {
    fn from(err: &AppError) -> Self {
        Self {
            code: err.code,
            message: err.message.clone(),
            hint: None,
            exit_code: None,
            stderr_tail: Vec::new(),
        }
    }
}

/// `profile://status-changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEvent {
    pub profile_id: String,
    pub status: ProfileStatus,
    /// Set while the process exists (`Starting`, `Running`, `Stopping`).
    pub pid: Option<u32>,
    /// Set with [`ProfileStatus::Error`].
    pub error: Option<LaunchFailure>,
}

/// `batch://progress`, emitted after each profile of a batch launch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchProgress {
    pub total: usize,
    /// Spawned (a browser that dies during startup still reports its own `Error` status).
    pub launched: usize,
    /// Could not be spawned; each also got an `Error` status event.
    pub failed: usize,
    /// Already running, or never tried because the app is quitting.
    pub skipped: usize,
}

/// Receives the launcher's events.
///
/// Called with the launcher's table locked (that is what keeps events in order): return
/// quickly, and never call back into the [`Launcher`].
pub trait StatusSink: Send + Sync + 'static {
    fn status_changed(&self, event: StatusEvent);
    fn batch_progress(&self, progress: BatchProgress);
}

/// A profile started by this launcher (`listRunning`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningInfo {
    pub profile_id: String,
    pub status: ProfileStatus,
    pub pid: u32,
    /// RFC 3339.
    pub started_at: String,
    pub kernel_id: String,
    pub user_data_dir: String,
}

/// Starts, watches and stops browser profiles. Cheap to clone; clones share one table. Must
/// be used inside a Tokio runtime.
#[derive(Clone)]
pub struct Launcher {
    inner: Arc<Inner>,
}

struct Inner {
    config: LauncherConfig,
    source: Arc<dyn LaunchSource>,
    sink: Arc<dyn StatusSink>,
    table: Mutex<Table>,
}

#[derive(Default)]
struct Table {
    slots: HashMap<String, Slot>,
    /// Set by [`Launcher::shutdown`]: no new launches.
    closed: bool,
}

enum Slot {
    /// Reserved while the profile is resolved and spawned, so it cannot be launched twice.
    Launching {
        stop_requested: bool,
    },
    Running(Entry),
}

struct Entry {
    pid: u32,
    status: ProfileStatus,
    started_at: String,
    kernel_id: String,
    user_data_dir: PathBuf,
    /// Taken by the first stop request.
    stop_tx: Option<oneshot::Sender<()>>,
    /// Becomes `true` once the waiter has finished.
    done: watch::Receiver<bool>,
}

impl Entry {
    fn info(&self, profile_id: &str) -> RunningInfo {
        RunningInfo {
            profile_id: profile_id.to_owned(),
            status: self.status,
            pid: self.pid,
            started_at: self.started_at.clone(),
            kernel_id: self.kernel_id.clone(),
            user_data_dir: self.user_data_dir.to_string_lossy().into_owned(),
        }
    }
}

impl Launcher {
    pub fn new(
        config: LauncherConfig,
        source: Arc<dyn LaunchSource>,
        sink: Arc<dyn StatusSink>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                source,
                sink,
                table: Mutex::new(Table::default()),
            }),
        }
    }

    /// Launches a profile (design §4.2) and returns once its process exists, in `Starting`.
    ///
    /// Errors: `profile_running` (already running or launching here; no event),
    /// `shutting_down` (no event), and — each also emitted as an `Error` status — whatever
    /// the source reports (`not_found`, `no_kernel`, ...), `kernel_missing`,
    /// `kernel_not_executable`, `conflict` (another running profile uses the same user data
    /// directory), `profile_running_externally`, `io` (directory not creatable) and
    /// `spawn_failed`.
    pub async fn launch(&self, profile_id: &str) -> AppResult<RunningInfo> {
        self.launch_with(profile_id, None).await
    }

    /// Asks a profile to stop (design §4.3) and waits until it has. No-op when it is not
    /// running. A profile still being spawned is stopped as soon as its process exists, without
    /// waiting here.
    pub async fn stop(&self, profile_id: &str) {
        if let Some(done) = self.request_stop(profile_id) {
            wait_done(done).await;
        }
    }

    /// [`Self::stop`] for several profiles at once (batch stop).
    pub async fn stop_many(&self, profile_ids: &[String]) {
        let waits: Vec<_> = profile_ids
            .iter()
            .filter_map(|id| self.request_stop(id))
            .collect();
        for done in waits {
            wait_done(done).await;
        }
    }

    /// Launches profiles in order (design §4.6): at most `concurrency` of them in `Starting` at
    /// once, `interval` before each spawn. A failure does not end the batch. Reports progress
    /// to the sink after each profile and returns the final count.
    pub async fn batch_launch(
        &self,
        profile_ids: &[String],
        concurrency: u32,
        interval: Duration,
    ) -> BatchProgress {
        let starting = Arc::new(Semaphore::new(concurrency.max(1) as usize));
        let mut progress = BatchProgress {
            total: profile_ids.len(),
            ..BatchProgress::default()
        };
        for (index, id) in profile_ids.iter().enumerate() {
            let Ok(permit) = Arc::clone(&starting).acquire_owned().await else {
                break; // never closed
            };
            tokio::time::sleep(interval).await;
            match self.launch_with(id, Some(permit)).await {
                Ok(_) => progress.launched += 1,
                Err(err) if err.code == ErrorCode::ProfileRunning => progress.skipped += 1,
                Err(err) if err.code == ErrorCode::ShuttingDown => {
                    progress.skipped += profile_ids.len() - index;
                    self.inner.sink.batch_progress(progress);
                    break;
                }
                Err(_) => progress.failed += 1,
            }
            self.inner.sink.batch_progress(progress);
        }
        progress
    }

    /// Stops every profile and refuses new launches; returns when nothing is left (the app's
    /// exit path, design §4.5).
    pub async fn shutdown(&self) {
        self.inner.table().closed = true;
        loop {
            let ids = self.running_ids();
            if ids.is_empty() {
                return;
            }
            self.stop_many(&ids).await;
            // Launches between reservation and spawn stop themselves once spawned (or fail).
            if !self.inner.table().slots.is_empty() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }

    /// Profiles with a process, ordered by start time.
    pub fn list_running(&self) -> Vec<RunningInfo> {
        let table = self.inner.table();
        let mut list: Vec<RunningInfo> = table
            .slots
            .iter()
            .filter_map(|(id, slot)| match slot {
                Slot::Running(entry) => Some(entry.info(id)),
                Slot::Launching { .. } => None,
            })
            .collect();
        list.sort_by(|a, b| (&a.started_at, &a.profile_id).cmp(&(&b.started_at, &b.profile_id)));
        list
    }

    /// Whether the profile is running or being launched here (e.g. to refuse deleting it).
    pub fn is_running(&self, profile_id: &str) -> bool {
        self.inner.table().slots.contains_key(profile_id)
    }

    /// Ids of profiles running or being launched here, for the `running` list view.
    pub fn running_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.inner.table().slots.keys().cloned().collect();
        ids.sort();
        ids
    }

    async fn launch_with(
        &self,
        profile_id: &str,
        permit: Option<OwnedSemaphorePermit>,
    ) -> AppResult<RunningInfo> {
        let reservation = Reservation::new(&self.inner, profile_id)?;

        let inner = Arc::clone(&self.inner);
        let id = profile_id.to_owned();
        let prepared = tokio::task::spawn_blocking(move || inner.prepare(&id))
            .await
            .unwrap_or_else(|err| Err(AppError::new(ErrorCode::Io, err.to_string())));
        let request = match prepared {
            Ok(request) => request,
            Err(err) => return Err(reservation.fail(err)),
        };
        let (tree, stderr) = match process::spawn(&request.plan.program, &request.plan.args) {
            Ok(spawned) => spawned,
            Err(err) => {
                return Err(reservation.fail(AppError::new(
                    ErrorCode::SpawnFailed,
                    format!("cannot start {}: {err}", request.plan.program.display()),
                )));
            }
        };

        let pid = tree.pid();
        let spawned_at = Instant::now();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (done_tx, done_rx) = watch::channel(false);
        let entry = Entry {
            pid,
            status: ProfileStatus::Starting,
            started_at: crate::db::now(),
            kernel_id: request.kernel_id,
            user_data_dir: request.user_data_dir,
            stop_tx: Some(stop_tx),
            done: done_rx,
        };
        let info = entry.info(profile_id);
        let log_file = self.log_file(profile_id, &entry);
        let stop_requested = reservation.promote(entry);

        let log = Arc::new(stderr::StderrLog::default());
        let reader = tokio::spawn(stderr::drain(stderr, Arc::clone(&log), log_file));
        tokio::spawn(waiter::run(waiter::Waiter {
            inner: Arc::clone(&self.inner),
            profile_id: profile_id.to_owned(),
            tree,
            stop_rx,
            done_tx,
            log,
            reader,
            permit,
            spawned_at,
        }));
        if stop_requested {
            self.request_stop(profile_id);
        }

        // `last_launched_at` is bookkeeping: the browser runs whether or not it is recorded.
        let source = Arc::clone(&self.inner.source);
        let id = profile_id.to_owned();
        let _ = tokio::task::spawn_blocking(move || source.launched(&id)).await;
        Ok(info)
    }

    /// `<log_dir>/<profile id>.log` and its header line, when logging is on and the id is safe
    /// as a file name (ids are UUIDs).
    fn log_file(&self, profile_id: &str, entry: &Entry) -> Option<(PathBuf, String)> {
        let dir = self.inner.config.log_dir.as_ref()?;
        let safe = !profile_id.is_empty()
            && profile_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        safe.then(|| {
            (
                dir.join(format!("{profile_id}.log")),
                format!("=== {} pid {} ===\n", entry.started_at, entry.pid),
            )
        })
    }

    /// Sends the stop request; returns the receiver that tells when the stop is complete.
    fn request_stop(&self, profile_id: &str) -> Option<watch::Receiver<bool>> {
        let mut table = self.inner.table();
        match table.slots.get_mut(profile_id)? {
            Slot::Launching { stop_requested } => {
                *stop_requested = true;
                None
            }
            Slot::Running(entry) => {
                if let Some(stop_tx) = entry.stop_tx.take() {
                    let _ = stop_tx.send(());
                }
                Some(entry.done.clone())
            }
        }
    }
}

async fn wait_done(mut done: watch::Receiver<bool>) {
    // `Err`: the waiter is gone, which also means done.
    let _ = done.wait_for(|done| *done).await;
}

impl Inner {
    fn table(&self) -> MutexGuard<'_, Table> {
        // Every critical section leaves the table consistent, so a poisoned lock is usable.
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(
        &self,
        profile_id: &str,
        status: ProfileStatus,
        pid: Option<u32>,
        error: Option<LaunchFailure>,
    ) {
        self.sink.status_changed(StatusEvent {
            profile_id: profile_id.to_owned(),
            status,
            pid,
            error,
        });
    }

    /// The blocking part of a launch (design §4.2 steps 1–3): resolve, check the executable,
    /// make sure no other browser holds the directory, create it.
    fn prepare(&self, profile_id: &str) -> AppResult<LaunchRequest> {
        let request = self.source.resolve(profile_id)?;
        check_executable(&request.plan.program)?;
        let dir = &request.user_data_dir;
        let in_use_by = self.table().slots.iter().find_map(|(id, slot)| match slot {
            Slot::Running(entry) if entry.user_data_dir == *dir && id != profile_id => {
                Some(id.clone())
            }
            _ => None,
        });
        if let Some(other) = in_use_by {
            return Err(AppError::conflict(format!(
                "user data directory {} is in use by running profile {other}",
                dir.display()
            )));
        }
        if let LockState::Held { pid } = lock::check(dir) {
            let by = pid.map_or_else(String::new, |pid| format!(" (pid {pid})"));
            return Err(AppError::new(
                ErrorCode::ProfileRunningExternally,
                format!(
                    "a browser started outside the launcher{by} is using {}",
                    dir.display()
                ),
            ));
        }
        std::fs::create_dir_all(dir).map_err(|err| {
            AppError::new(
                ErrorCode::Io,
                format!("cannot create user data directory {}: {err}", dir.display()),
            )
        })?;
        Ok(request)
    }

    /// Waiter: a status change of a running profile.
    fn set_status(&self, profile_id: &str, status: ProfileStatus) {
        let mut table = self.table();
        if let Some(Slot::Running(entry)) = table.slots.get_mut(profile_id) {
            entry.status = status;
            self.emit(profile_id, status, Some(entry.pid), None);
        }
    }

    /// Waiter: the browser is gone.
    fn finish(&self, profile_id: &str, status: ProfileStatus, error: Option<LaunchFailure>) {
        let mut table = self.table();
        table.slots.remove(profile_id);
        self.emit(profile_id, status, None, error);
    }
}

/// The kernel must be an existing file; on Unix also executable (a missing `x` bit is the
/// usual state of a freshly extracted download).
fn check_executable(program: &Path) -> AppResult<()> {
    let not_executable = |why: String| {
        AppError::new(
            ErrorCode::KernelNotExecutable,
            format!("kernel {} {why}", program.display()),
        )
    };
    let meta = match std::fs::metadata(program) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::new(
                ErrorCode::KernelMissing,
                format!("kernel not found: {}", program.display()),
            ));
        }
        Err(err) => return Err(not_executable(format!("cannot be read: {err}"))),
    };
    if !meta.is_file() {
        return Err(not_executable("is not a file".to_owned()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(not_executable("is not executable (chmod +x)".to_owned()));
        }
    }
    Ok(())
}

/// A [`Slot::Launching`] reservation. Dropping it (a failed or cancelled launch) frees the
/// slot.
struct Reservation<'a> {
    inner: &'a Inner,
    profile_id: &'a str,
    armed: bool,
}

impl<'a> Reservation<'a> {
    fn new(inner: &'a Inner, profile_id: &'a str) -> AppResult<Self> {
        let mut table = inner.table();
        if table.closed {
            return Err(AppError::new(
                ErrorCode::ShuttingDown,
                "the launcher is shutting down",
            ));
        }
        if table.slots.contains_key(profile_id) {
            return Err(AppError::new(
                ErrorCode::ProfileRunning,
                format!("profile {profile_id} is already running"),
            ));
        }
        table.slots.insert(
            profile_id.to_owned(),
            Slot::Launching {
                stop_requested: false,
            },
        );
        Ok(Self {
            inner,
            profile_id,
            armed: true,
        })
    }

    /// Frees the slot and emits the failure, atomically; returns `err` for the caller.
    fn fail(mut self, err: AppError) -> AppError {
        let mut table = self.inner.table();
        table.slots.remove(self.profile_id);
        self.inner.emit(
            self.profile_id,
            ProfileStatus::Error,
            None,
            Some(LaunchFailure::from(&err)),
        );
        self.armed = false;
        err
    }

    /// Replaces the reservation with the running entry and emits `Starting`. Returns whether a
    /// stop was requested in the meantime.
    fn promote(mut self, entry: Entry) -> bool {
        let mut table = self.inner.table();
        let pid = entry.pid;
        let previous = table
            .slots
            .insert(self.profile_id.to_owned(), Slot::Running(entry));
        self.inner
            .emit(self.profile_id, ProfileStatus::Starting, Some(pid), None);
        self.armed = false;
        matches!(
            previous,
            Some(Slot::Launching {
                stop_requested: true
            })
        )
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.inner.table().slots.remove(self.profile_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The shapes the frontend receives (design §5).
    #[test]
    fn serializes_for_the_bridge() {
        let event = StatusEvent {
            profile_id: "p".into(),
            status: ProfileStatus::Error,
            pid: None,
            error: Some(LaunchFailure {
                code: ErrorCode::StartupFailed,
                message: "the browser exited during startup (exit status: 1)".into(),
                hint: Some(FailureHint::NoSandbox),
                exit_code: Some(1),
                stderr_tail: vec!["No usable sandbox!".into()],
            }),
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "profileId": "p",
                "status": "error",
                "pid": null,
                "error": {
                    "code": "startup_failed",
                    "message": "the browser exited during startup (exit status: 1)",
                    "hint": "no_sandbox",
                    "exitCode": 1,
                    "stderrTail": ["No usable sandbox!"],
                },
            })
        );
        for (status, text) in [
            (ProfileStatus::Stopped, "stopped"),
            (ProfileStatus::Starting, "starting"),
            (ProfileStatus::Running, "running"),
            (ProfileStatus::Stopping, "stopping"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), text);
        }
        assert_eq!(
            serde_json::to_value(FailureHint::MissingFuse).unwrap(),
            "missing_fuse"
        );
        assert_eq!(
            serde_json::to_value(BatchProgress {
                total: 4,
                launched: 2,
                failed: 1,
                skipped: 1
            })
            .unwrap(),
            json!({ "total": 4, "launched": 2, "failed": 1, "skipped": 1 })
        );
        let info = RunningInfo {
            profile_id: "p".into(),
            status: ProfileStatus::Running,
            pid: 42,
            started_at: "2026-10-13T00:00:00.000Z".into(),
            kernel_id: "k".into(),
            user_data_dir: "/data/p".into(),
        };
        assert_eq!(
            serde_json::to_value(info).unwrap(),
            json!({
                "profileId": "p",
                "status": "running",
                "pid": 42,
                "startedAt": "2026-10-13T00:00:00.000Z",
                "kernelId": "k",
                "userDataDir": "/data/p",
            })
        );
    }

    #[test]
    fn pre_spawn_errors_become_failures_without_process_details() {
        let failure = LaunchFailure::from(&AppError::new(ErrorCode::KernelMissing, "gone"));
        assert_eq!(failure.code, ErrorCode::KernelMissing);
        assert_eq!(failure.message, "gone");
        assert_eq!((failure.hint, failure.exit_code), (None, None));
        assert!(failure.stderr_tail.is_empty());
    }
}

//! One waiter task per spawned browser (design §4.2 / §4.3). After `Starting`, every status
//! change of the profile happens here, so a stop request racing a natural exit, or an exit
//! racing the end of the startup grace period, is decided in exactly one place: whichever
//! `select!` branch completes first.
//!
//! | Event | Status |
//! |---|---|
//! | still alive after `startup_grace` | Starting → Running |
//! | exits within `startup_grace` (any code) | Error |
//! | exits with a failure within `early_exit_window` | Error |
//! | exits at any other time (the user closed it) | Stopped |
//! | stop requested | Stopping → (close, `stop_grace`, kill) → Stopped |

use std::io;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, oneshot, watch};
use tokio::task::JoinHandle;

use super::stderr::StderrLog;
use super::{Inner, LaunchFailure, LauncherConfig, ProfileStatus};
use crate::error::ErrorCode;
use crate::kernel::process::ProcessTree;

/// How long the stderr reader may take to reach EOF after the process tree is gone.
const READER_DRAIN: Duration = Duration::from_secs(2);
/// stderr lines quoted in an error message (the event carries the whole tail).
const MESSAGE_TAIL_LINES: usize = 5;

pub(super) struct Waiter {
    pub(super) inner: Arc<Inner>,
    pub(super) profile_id: String,
    pub(super) tree: ProcessTree,
    pub(super) stop_rx: oneshot::Receiver<()>,
    pub(super) done_tx: watch::Sender<bool>,
    pub(super) log: Arc<StderrLog>,
    pub(super) reader: JoinHandle<()>,
    /// A batch launch slot, held while the profile is `Starting` (design §4.6).
    pub(super) permit: Option<OwnedSemaphorePermit>,
    pub(super) spawned_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Starting,
    Running,
    /// Asked to close; killed when the grace period ends.
    Stopping,
    /// Killed; waiting for the main process to be reaped.
    Killing,
}

pub(super) async fn run(waiter: Waiter) {
    let Waiter {
        inner,
        profile_id: id,
        mut tree,
        mut stop_rx,
        done_tx,
        log,
        mut reader,
        mut permit,
        spawned_at,
    } = waiter;
    let config = &inner.config;
    let startup = tokio::time::sleep(config.startup_grace);
    let kill_deadline = tokio::time::sleep(config.stop_grace);
    tokio::pin!(startup, kill_deadline);
    let mut phase = Phase::Starting;
    let mut stop_open = true;

    let exit = loop {
        tokio::select! {
            exit = tree.wait() => break exit,
            () = &mut startup, if phase == Phase::Starting => {
                phase = Phase::Running;
                inner.set_status(&id, ProfileStatus::Running);
                // After the event, so a batch never shows more profiles starting than allowed.
                drop(permit.take());
            }
            request = &mut stop_rx, if stop_open => {
                stop_open = false;
                // `Err`: the sender was dropped without a request.
                if request.is_ok() {
                    phase = Phase::Stopping;
                    inner.set_status(&id, ProfileStatus::Stopping);
                    drop(permit.take());
                    kill_deadline
                        .as_mut()
                        .reset(tokio::time::Instant::now() + config.stop_grace);
                    if tree.request_close().is_err() {
                        // Could not even ask; force at once.
                        let _ = tree.kill_tree();
                        phase = Phase::Killing;
                    }
                }
            }
            () = &mut kill_deadline, if phase == Phase::Stopping => {
                let _ = tree.kill_tree();
                phase = Phase::Killing;
            }
        }
    };

    // Whatever the main process left behind goes with it: children orphaned by a crash or by
    // an external kill of the main process. Nothing is left after a clean exit.
    let _ = tree.kill_tree();
    drop(tree);
    // With the tree gone nobody holds the pipe any more; let the reader see EOF so the tail is
    // complete.
    if tokio::time::timeout(READER_DRAIN, &mut reader)
        .await
        .is_err()
    {
        reader.abort();
    }

    let failure = classify(phase, &exit, spawned_at.elapsed(), config, &log);
    let status = if failure.is_some() {
        ProfileStatus::Error
    } else {
        ProfileStatus::Stopped
    };
    inner.finish(&id, status, failure);
    drop(permit);
    let _ = done_tx.send(true);
}

/// `Some` when the exit is a launch failure rather than a stop.
fn classify(
    phase: Phase,
    exit: &io::Result<ExitStatus>,
    lifetime: Duration,
    config: &LauncherConfig,
    log: &StderrLog,
) -> Option<LaunchFailure> {
    match (phase, exit) {
        (Phase::Stopping | Phase::Killing, _) => None,
        (_, Err(err)) => Some(failure(
            ErrorCode::Io,
            format!("lost track of the browser process: {err}"),
            None,
            log,
        )),
        (Phase::Starting, Ok(status)) => Some(startup_failure(*status, log)),
        (Phase::Running, Ok(status))
            if !status.success() && lifetime < config.early_exit_window =>
        {
            Some(startup_failure(*status, log))
        }
        (Phase::Running, Ok(_)) => None,
    }
}

fn startup_failure(status: ExitStatus, log: &StderrLog) -> LaunchFailure {
    failure(
        ErrorCode::StartupFailed,
        format!("the browser exited during startup ({status})"),
        status.code(),
        log,
    )
}

fn failure(
    code: ErrorCode,
    summary: String,
    exit_code: Option<i32>,
    log: &StderrLog,
) -> LaunchFailure {
    let stderr_tail = log.tail();
    let quoted = &stderr_tail[stderr_tail.len().saturating_sub(MESSAGE_TAIL_LINES)..];
    let message = if quoted.is_empty() {
        summary
    } else {
        format!("{summary}:\n{}", quoted.join("\n"))
    };
    LaunchFailure {
        code,
        message,
        hint: log.hint(),
        exit_code,
        stderr_tail,
    }
}

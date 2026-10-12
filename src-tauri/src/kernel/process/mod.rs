//! A spawned kernel together with every process it starts (design §4.3 / §4.4).
//!
//! Chrome forks GPU, network and renderer processes. Killing only the main process would leave
//! them behind, so the whole tree is tracked from the moment of the spawn:
//!
//! - Unix (Linux, macOS): the kernel leads a new process group (`process_group(0)`, pgid = pid)
//!   and the tree is killed with `killpg`.
//! - Windows: the kernel is assigned to a Job Object with kill-on-close right after the spawn,
//!   so the tree also dies when the launcher exits or crashes. Processes the kernel starts
//!   before the assignment (it is not suspended in between) would escape the job; Chrome
//!   needs far longer than that to start its first child.
//!
//! Stopping is two-step: [`ProcessTree::request_close`] asks the browser to close (it flushes
//! cookies and other state to disk on the way out), [`ProcessTree::kill_tree`] forces it.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use self::unix as imp;
#[cfg(windows)]
use self::windows as imp;

use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use tokio::process::{Child, ChildStderr, Command};

#[cfg(unix)]
pub(crate) use self::unix::pid_alive;

/// A running kernel and its descendants.
#[derive(Debug)]
pub struct ProcessTree {
    child: Child,
    pid: u32,
    tree: imp::Tree,
}

/// Starts `program` with `args` (passed as argv, no shell). stdin and stdout are discarded;
/// stderr is returned piped, and the caller must keep reading it: a full pipe blocks the
/// browser.
pub fn spawn(program: &Path, args: &[String]) -> io::Result<(ProcessTree, ChildStderr)> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut tree = imp::Tree::prepare(&mut command)?;

    let mut child = command.spawn()?;
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("the process exited before its pid was read"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr was not captured"))?;
    if let Err(err) = tree.adopt(&child, pid) {
        // Without the tree handle the browser could not be stopped as a whole: refuse rather
        // than leak it.
        let _ = child.start_kill();
        return Err(err);
    }

    Ok((ProcessTree { child, pid, tree }, stderr))
}

impl ProcessTree {
    /// The main process id (also the process group id on Unix).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Waits for the main process to exit and reaps it. Cancel-safe.
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }

    /// Asks the browser to close: `SIGTERM` to the main process only (it shuts its children
    /// down itself; signalling the whole group would kill renderers first), or `taskkill`
    /// without `/F`, which posts `WM_CLOSE` to its windows. Returns before the browser exits.
    pub fn request_close(&self) -> io::Result<()> {
        imp::request_close(self.pid)
    }

    /// Kills every process of the tree, including the main process. Succeeds when nothing is
    /// left to kill, so it also sweeps up after the main process has exited.
    pub fn kill_tree(&self) -> io::Result<()> {
        self.tree.kill()
    }
}

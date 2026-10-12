//! Unix (Linux, macOS): the kernel leads its own process group.

use std::io;

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::Pid;
use tokio::process::{Child, Command};

/// The kernel's process group.
#[derive(Debug, Default)]
pub(super) struct Tree {
    pgid: Option<Pid>,
}

impl Tree {
    pub(super) fn prepare(command: &mut Command) -> io::Result<Self> {
        // 0: a new group whose id is the child's pid. Every process Chrome forks inherits it.
        command.process_group(0);
        Ok(Self::default())
    }

    pub(super) fn adopt(&mut self, _child: &Child, pid: u32) -> io::Result<()> {
        self.pgid = Some(to_pid(pid)?);
        Ok(())
    }

    /// `SIGKILL` to the whole group. Also correct after the leader has been reaped: POSIX does
    /// not reuse a pid while a process group with that id still exists, and an empty group
    /// yields `ESRCH`, which counts as success. (macOS skips zombies and reports `EPERM` while
    /// only zombies are left; callers treat every error as "nothing more to do".)
    pub(super) fn kill(&self) -> io::Result<()> {
        let Some(pgid) = self.pgid else {
            return Ok(());
        };
        match killpg(pgid, Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(errno) => Err(errno.into()),
        }
    }
}

pub(super) fn request_close(pid: u32) -> io::Result<()> {
    Ok(kill(to_pid(pid)?, Signal::SIGTERM)?)
}

/// Whether a process with this id exists. `EPERM` means it does but belongs to another user.
pub(crate) fn pid_alive(pid: u32) -> bool {
    to_pid(pid).is_ok_and(|pid| matches!(kill(pid, None), Ok(()) | Err(Errno::EPERM)))
}

/// Rejects 0 and values that are not a valid `pid_t`: `kill(0, ...)` would signal the
/// launcher's own process group, and a negative pid a whole group.
fn to_pid(pid: u32) -> io::Result<Pid> {
    match i32::try_from(pid) {
        Ok(raw) if raw > 0 => Ok(Pid::from_raw(raw)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a process id: {pid}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_alive_for_own_process_only() {
        assert!(pid_alive(std::process::id()));
        // Never "alive" by accident: 0 would address our own group.
        assert!(!pid_alive(0));
        assert!(!pid_alive(u32::MAX));
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        // Reaped; the pid could in theory be reused, but not within this test.
        assert!(!pid_alive(pid));
    }

    #[test]
    fn invalid_pids_are_errors() {
        assert!(request_close(0).is_err());
        assert!(to_pid(i32::MAX as u32 + 1).is_err());
        assert_eq!(to_pid(1).unwrap(), Pid::from_raw(1));
    }
}

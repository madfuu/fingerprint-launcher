//! Single-instance lock detection (design §4.2 step 2, research/process-lifecycle.md §5).
//!
//! Chromium allows one browser per user data directory. A second process started on the same
//! directory hands its command line to the running one and exits at once with code 0, which
//! would leave the launcher reporting "stopped" for a browser that is open. So before a spawn,
//! the launcher checks whether a browser it did not start holds the directory.

use std::path::Path;

/// Who holds a user data directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    /// No browser: no lock, or one left behind by a browser that is gone.
    Free,
    /// A live browser holds the directory. `pid` is known on Unix only.
    Held { pid: Option<u32> },
}

/// Inspects the lock of `user_data_dir`. Anything unexpected (unreadable lock, malformed
/// target) counts as free: the browser itself then has the final word.
pub fn check(user_data_dir: &Path) -> LockState {
    imp::check(user_data_dir)
}

/// Linux / macOS: `SingletonLock` is a symlink to `<hostname>-<pid>` of the browser that
/// created it. It is held when it names this host and that process is alive (a lock from
/// another host is on a shared directory and cannot be checked; Chromium itself treats it as
/// stale). A recycled pid makes a stale lock look held; the error then names the pid.
#[cfg(unix)]
mod imp {
    use std::path::Path;

    use super::LockState;
    use crate::kernel::process::pid_alive;

    pub(super) const LOCK_FILE: &str = "SingletonLock";

    pub(super) fn check(user_data_dir: &Path) -> LockState {
        let Ok(target) = std::fs::read_link(user_data_dir.join(LOCK_FILE)) else {
            return LockState::Free;
        };
        match parse_target(&target.to_string_lossy()) {
            Some((host, pid)) if is_this_host(host) && pid_alive(pid) => {
                LockState::Held { pid: Some(pid) }
            }
            _ => LockState::Free,
        }
    }

    /// `<hostname>-<pid>`; host names may contain `-`, so the pid is after the last one.
    pub(super) fn parse_target(target: &str) -> Option<(&str, u32)> {
        let (host, pid) = target.rsplit_once('-')?;
        Some((host, pid.parse().ok()?))
    }

    fn is_this_host(host: &str) -> bool {
        // Chromium writes the name from gethostname(); if ours is unavailable, assume a match
        // rather than ignore a lock that may be live.
        nix::unistd::gethostname()
            .map(|ours| ours.to_string_lossy() == host)
            .unwrap_or(true)
    }
}

/// Windows: a running browser keeps `lockfile` open for writing and shares it for reading
/// only (and deletes it on close). Opening it without sharing anything then fails with a
/// sharing violation exactly while a browser holds it.
#[cfg(windows)]
mod imp {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;

    use super::LockState;

    pub(super) const LOCK_FILE: &str = "lockfile";

    pub(super) fn check(user_data_dir: &Path) -> LockState {
        let opened = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(user_data_dir.join(LOCK_FILE));
        match opened {
            Err(err) if err.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => {
                LockState::Held { pid: None }
            }
            // Our handle (if any) closes right here.
            _ => LockState::Free,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_lock_is_free() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check(dir.path()), LockState::Free);
        assert_eq!(check(&dir.path().join("does-not-exist")), LockState::Free);
    }

    #[cfg(unix)]
    mod unix {
        use std::os::unix::fs::symlink;

        use super::super::imp::{LOCK_FILE, parse_target};
        use super::*;

        fn hostname() -> String {
            nix::unistd::gethostname()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        }

        fn lock_to(target: &str) -> tempfile::TempDir {
            let dir = tempfile::tempdir().unwrap();
            symlink(target, dir.path().join(LOCK_FILE)).unwrap();
            dir
        }

        #[test]
        fn parses_host_and_pid() {
            assert_eq!(parse_target("box-1234"), Some(("box", 1234)));
            assert_eq!(parse_target("my-host-name-42"), Some(("my-host-name", 42)));
            for bad in ["", "box", "box-", "box-x1", "box-1.5", "box- 1"] {
                assert_eq!(parse_target(bad), None, "{bad}");
            }
        }

        #[test]
        fn live_process_on_this_host_holds_the_lock() {
            let pid = std::process::id();
            let dir = lock_to(&format!("{}-{pid}", hostname()));
            assert_eq!(check(dir.path()), LockState::Held { pid: Some(pid) });
        }

        #[test]
        fn stale_locks_are_free() {
            let mut child = std::process::Command::new("true").spawn().unwrap();
            let dead = child.id();
            child.wait().unwrap();
            let own = std::process::id();
            for target in [
                format!("{}-{dead}", hostname()),
                format!("{}-elsewhere-{own}", hostname()),
                format!("other-host-{own}"),
                "garbage".to_owned(),
                format!("{}-0", hostname()),
            ] {
                let dir = lock_to(&target);
                assert_eq!(check(dir.path()), LockState::Free, "{target}");
            }
        }

        #[test]
        fn regular_file_is_not_a_lock() {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(LOCK_FILE), "x").unwrap();
            assert_eq!(check(dir.path()), LockState::Free);
        }
    }

    #[cfg(windows)]
    mod windows {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;

        use super::super::imp::LOCK_FILE;
        use super::*;

        /// `FILE_SHARE_READ`, the share mode Chromium opens `lockfile` with.
        const FILE_SHARE_READ: u32 = 1;

        #[test]
        fn open_lockfile_is_held_and_closed_one_is_free() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(LOCK_FILE);
            let held = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .share_mode(FILE_SHARE_READ)
                .open(&path)
                .unwrap();
            assert_eq!(check(dir.path()), LockState::Held { pid: None });
            drop(held);
            // Left behind without a holder: stale.
            assert!(path.exists());
            assert_eq!(check(dir.path()), LockState::Free);
        }
    }
}

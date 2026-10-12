//! Windows: a Job Object per kernel, and `taskkill` to ask it to close.
//!
//! Signatures follow windows-sys 0.61 (`Win32::System::JobObjects`): the calls return `BOOL`
//! (0 = failure, details in `GetLastError`) and `CreateJobObjectW` returns NULL on failure.

use std::io;
use std::os::windows::process::CommandExt;
use std::process::{Command as StdCommand, Stdio};
use std::ptr;

use tokio::process::{Child, Command};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_sys::core::BOOL;

/// A Job Object with kill-on-close: closing the last handle (on drop, or when the launcher
/// exits or crashes) terminates every process in it.
#[derive(Debug)]
pub(super) struct Tree {
    job: HANDLE,
}

// SAFETY: a job object handle is a kernel handle; every call made on it here is valid from any
// thread, and it is closed exactly once, in `drop`.
unsafe impl Send for Tree {}
// SAFETY: as above; `&self` methods only pass the handle to thread-safe kernel calls.
unsafe impl Sync for Tree {}

impl Tree {
    pub(super) fn prepare(_command: &mut Command) -> io::Result<Self> {
        // SAFETY: both arguments may be null: default security attributes, anonymous job.
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        // Owned from here on, so an error below closes it.
        let tree = Self { job };
        let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: `info` is a live JOBOBJECT_EXTENDED_LIMIT_INFORMATION, which is what this
        // information class expects, and the length is its size.
        check(unsafe {
            SetInformationJobObject(
                tree.job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of_val(&info) as u32,
            )
        })?;
        Ok(tree)
    }

    pub(super) fn adopt(&mut self, child: &Child, _pid: u32) -> io::Result<()> {
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the process exited before it joined its job"))?;
        // SAFETY: both handles are open: the job is owned by `self`, the process handle by
        // `child`, which outlives this call.
        check(unsafe { AssignProcessToJobObject(self.job, process) })
    }

    pub(super) fn kill(&self) -> io::Result<()> {
        // SAFETY: the job handle stays open until `drop`.
        check(unsafe { TerminateJobObject(self.job, 1) })
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: owned handle, closed only here.
        unsafe { CloseHandle(self.job) };
    }
}

fn check(ok: BOOL) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// `taskkill /PID <pid>`: without `/F` it posts `WM_CLOSE` to the process's windows, which is
/// how Chrome is closed gracefully; without `/T` only the main process is asked. A process with
/// no window (e.g. headless) refuses, and the caller's grace period ends in [`Tree::kill`].
/// Fire and forget: the result arrives as the browser's exit.
pub(super) fn request_close(pid: u32) -> io::Result<()> {
    StdCommand::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // The launcher has no console; without this every stop would flash one.
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(drop)
}

//! The browser's stderr: read for the whole life of the process (a full pipe would block the
//! browser), kept as a short tail for error messages, scanned for known startup problems, and
//! appended to a per-profile log file (design §4.2 step 7).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::ChildStderr;

use super::FailureHint;

/// Lines kept for error messages.
pub const STDERR_TAIL_LINES: usize = 50;
/// Longer lines are cut (the log file keeps them whole).
const MAX_LINE_BYTES: usize = 4096;
/// A log larger than this at launch is started afresh.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// What has been read so far.
#[derive(Debug, Default)]
pub(super) struct StderrLog {
    state: Mutex<LogState>,
}

#[derive(Debug, Default)]
struct LogState {
    tail: VecDeque<String>,
    hint: Option<FailureHint>,
}

impl StderrLog {
    fn state(&self) -> MutexGuard<'_, LogState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The last [`STDERR_TAIL_LINES`] lines, oldest first.
    pub(super) fn tail(&self) -> Vec<String> {
        self.state().tail.iter().cloned().collect()
    }

    /// The first known problem seen anywhere in the output (it may have scrolled out of the
    /// tail).
    pub(super) fn hint(&self) -> Option<FailureHint> {
        self.state().hint
    }

    fn push_line(&self, bytes: &[u8]) {
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        let line = String::from_utf8_lossy(bytes).into_owned();
        let mut state = self.state();
        if state.hint.is_none() {
            state.hint = hint_for(&line);
        }
        if state.tail.len() == STDERR_TAIL_LINES {
            state.tail.pop_front();
        }
        state.tail.push_back(line);
    }
}

/// Known startup problems and what the user can do about them (research/process-lifecycle.md
/// §4.2).
fn hint_for(line: &str) -> Option<FailureHint> {
    if line.contains("No usable sandbox") {
        // Ubuntu 23.10+ restricts unprivileged user namespaces through AppArmor.
        Some(FailureHint::NoSandbox)
    } else if line.contains("libfuse.so.2") || line.contains("AppImages require FUSE") {
        Some(FailureHint::MissingFuse)
    } else {
        None
    }
}

/// Reads `stderr` to its end into `log`, copying the raw bytes to `log_file` (path and a header
/// line) when given. Read errors end the loop like EOF; write errors only stop the file copy.
pub(super) async fn drain(
    mut stderr: ChildStderr,
    log: std::sync::Arc<StderrLog>,
    log_file: Option<(PathBuf, String)>,
) {
    let mut file = match log_file {
        Some((path, header)) => open_log(&path, &header).await,
        None => None,
    };
    let mut chunk = vec![0; 8192];
    let mut line = Vec::new();
    loop {
        let read = match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let data = &chunk[..read];
        if let Some(out) = file.as_mut()
            && out.write_all(data).await.is_err()
        {
            file = None;
        }
        let mut rest = data;
        while let Some(newline) = rest.iter().position(|&b| b == b'\n') {
            push_capped(&mut line, &rest[..newline]);
            log.push_line(&line);
            line.clear();
            rest = &rest[newline + 1..];
        }
        push_capped(&mut line, rest);
    }
    if !line.is_empty() {
        log.push_line(&line);
    }
    if let Some(mut out) = file {
        let _ = out.flush().await;
    }
}

fn push_capped(line: &mut Vec<u8>, bytes: &[u8]) {
    let room = MAX_LINE_BYTES.saturating_sub(line.len());
    line.extend_from_slice(&bytes[..bytes.len().min(room)]);
}

/// Opens the log for appending (or afresh when it has grown past [`MAX_LOG_BYTES`]) and writes
/// `header`. `None` if it cannot be written: logging is best effort.
async fn open_log(path: &Path, header: &str) -> Option<tokio::fs::File> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.ok()?;
    }
    let too_big = tokio::fs::metadata(path)
        .await
        .is_ok_and(|meta| meta.len() > MAX_LOG_BYTES);
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!too_big)
        .truncate(too_big)
        .open(path)
        .await
        .ok()?;
    file.write_all(header.as_bytes()).await.ok()?;
    Some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_last_lines_and_the_first_hint() {
        let log = StderrLog::default();
        log.push_line(b"[1:2:ERROR:zygote_host_impl_linux.cc(127)] No usable sandbox! ...\r");
        for i in 0..STDERR_TAIL_LINES + 10 {
            log.push_line(format!("line {i}").as_bytes());
        }
        log.push_line(b"dlopen(): error loading libfuse.so.2");
        let tail = log.tail();
        assert_eq!(tail.len(), STDERR_TAIL_LINES);
        assert_eq!(tail[0], "line 11");
        assert_eq!(tail.last().unwrap(), "dlopen(): error loading libfuse.so.2");
        assert_eq!(log.hint(), Some(FailureHint::NoSandbox));
    }

    #[test]
    fn hints() {
        assert_eq!(
            hint_for("AppImages require FUSE to run."),
            Some(FailureHint::MissingFuse)
        );
        assert_eq!(hint_for("all good"), None);
    }

    #[test]
    fn invalid_utf8_and_long_lines() {
        let log = StderrLog::default();
        log.push_line(b"bad \xff byte");
        assert_eq!(log.tail(), ["bad \u{fffd} byte"]);

        let mut line = Vec::new();
        push_capped(&mut line, &[b'x'; MAX_LINE_BYTES - 1]);
        push_capped(&mut line, b"abc");
        assert_eq!(line.len(), MAX_LINE_BYTES);
        assert_eq!(line.last(), Some(&b'a'));
    }
}

//! The fake kernel: this test binary, started by the launcher instead of Chromium.
//!
//! Recognised by its arguments before any test runs (see `main`):
//!
//! - any `--user-data-dir=<dir>` argument: behave like a browser on `<dir>`, configured by
//!   `<dir>/fake.cfg`;
//! - `--fake-chrome-child <dir>`: the browser's child process (renderer stand-in);
//! - exactly `--version`: print a Chromium version line.
//!
//! What the browser does, in order: write its argv (`argv.json`) and pid (`main.pid`), print
//! the configured stderr lines, flood stderr if asked, exit at once if `exit-after 0`, ignore
//! `SIGTERM` if asked (Unix), take the single-instance lock if asked, start its child (which
//! inherits stderr, like Chrome's children), write `ready`, then idle until killed or until
//! `exit-after` elapses. The child writes `child.pid` and bumps `child.beat` every 50 ms, so
//! tests can tell whether it is alive on any OS.
//!
//! Both exit on their own once the directory disappears or after [`MAX_LIFETIME`], so a broken
//! test cannot leak processes for long.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const CHILD_FLAG: &str = "--fake-chrome-child";
pub const CONFIG_FILE: &str = "fake.cfg";
pub const VERSION_LINE: &str = "Chromium 150.0.7871.186 fake build";
const MAX_LIFETIME: Duration = Duration::from_secs(120);

/// `fake.cfg`: one switch per line.
#[derive(Debug, Default)]
pub struct Config {
    /// `exit-after <ms>`: exit by itself (0 = right after startup output).
    pub exit_after: Option<Duration>,
    /// `exit-code <n>`, used by `exit-after`.
    pub exit_code: i32,
    /// `stderr <text>`, repeatable.
    pub stderr: Vec<String>,
    /// `stderr-flood <KiB>`: write this much to stderr before `ready`.
    pub stderr_flood_kib: usize,
    /// `ignore-term`: ignore `SIGTERM` (Unix).
    pub ignore_term: bool,
    /// `lock`: hold the single-instance lock like Chromium.
    pub lock: bool,
    /// `no-child`: do not start a child.
    pub no_child: bool,
}

impl Config {
    fn read(dir: &Path) -> Self {
        let mut config = Self::default();
        let text = fs::read_to_string(dir.join(CONFIG_FILE)).unwrap_or_default();
        for line in text.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "exit-after" => {
                    config.exit_after = Some(Duration::from_millis(value.parse().unwrap()));
                }
                "exit-code" => config.exit_code = value.parse().unwrap(),
                "stderr" => config.stderr.push(value.to_owned()),
                "stderr-flood" => config.stderr_flood_kib = value.parse().unwrap(),
                "ignore-term" => config.ignore_term = true,
                "lock" => config.lock = true,
                "no-child" => config.no_child = true,
                "" => {}
                other => panic!("unknown fake.cfg switch {other}"),
            }
        }
        config
    }
}

/// Runs the fake if `args` ask for it; never returns then.
pub fn dispatch(args: &[String]) {
    if args.get(1).map(String::as_str) == Some(CHILD_FLAG) {
        run_child(Path::new(&args[2]));
    }
    if args.len() == 2 && args[1] == "--version" {
        println!("{VERSION_LINE}");
        std::process::exit(0);
    }
    if let Some(dir) = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--user-data-dir="))
    {
        run_browser(Path::new(dir), &args[1..]);
    }
}

fn run_browser(dir: &Path, args: &[String]) -> ! {
    fs::create_dir_all(dir).unwrap();
    let config = Config::read(dir);
    write(dir, "argv.json", &serde_json::to_string(args).unwrap());
    write(dir, "main.pid", &std::process::id().to_string());

    let mut stderr = std::io::stderr().lock();
    for line in &config.stderr {
        writeln!(stderr, "{line}").unwrap();
    }
    let filler = format!("{}\n", "x".repeat(1023));
    for _ in 0..config.stderr_flood_kib {
        stderr.write_all(filler.as_bytes()).unwrap();
    }
    stderr.flush().unwrap();
    drop(stderr);
    if config.exit_after == Some(Duration::ZERO) {
        std::process::exit(config.exit_code);
    }

    if config.ignore_term {
        ignore_sigterm();
    }
    let _lock = config.lock.then(|| take_lock(dir));
    if !config.no_child {
        // Never waited for, like a browser's renderer when the browser is killed: reaping the
        // orphan is up to the OS, and cleaning it up is up to the launcher under test.
        #[allow(clippy::zombie_processes)]
        Command::new(std::env::current_exe().unwrap())
            .arg(CHILD_FLAG)
            .arg(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
    }
    write(dir, "ready", "");

    let started = Instant::now();
    loop {
        if config
            .exit_after
            .is_some_and(|after| started.elapsed() >= after)
        {
            std::process::exit(config.exit_code);
        }
        if started.elapsed() > MAX_LIFETIME || !dir.exists() {
            std::process::exit(99);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_child(dir: &Path) -> ! {
    write(dir, "child.pid", &std::process::id().to_string());
    let started = Instant::now();
    for beat in 0u64.. {
        // A failed write (e.g. a virus scanner holding the file) only skips one beat.
        let _ = fs::write(dir.join("child.beat"), beat.to_string());
        if started.elapsed() > MAX_LIFETIME || !dir.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::process::exit(99);
}

/// Writes a status file, retrying briefly: on Windows a virus scanner may hold a new file.
fn write(dir: &Path, name: &str, contents: &str) {
    let path = dir.join(name);
    for _ in 0..40 {
        if fs::write(&path, contents).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    fs::write(&path, contents).unwrap();
}

#[cfg(unix)]
fn ignore_sigterm() {
    use nix::sys::signal::{SigHandler, Signal, signal};
    // SAFETY: installs SIG_IGN, so no handler code ever runs; nothing else here uses signals.
    unsafe { signal(Signal::SIGTERM, SigHandler::SigIgn) }.unwrap();
}

/// Windows has no SIGTERM. The launcher's polite close there is `taskkill`, which a process
/// without windows (like this one) always refuses, so every stop already ends in the kill.
#[cfg(windows)]
fn ignore_sigterm() {}

/// Chromium's lock: a `SingletonLock` symlink to `<host>-<pid>` on Unix; `lockfile` opened for
/// writing and shared for reading only on Windows, where the returned handle holds it.
#[cfg(unix)]
fn take_lock(dir: &Path) {
    let host = nix::unistd::gethostname().unwrap();
    let target = format!("{}-{}", host.to_string_lossy(), std::process::id());
    let path = dir.join("SingletonLock");
    let _ = fs::remove_file(&path);
    std::os::unix::fs::symlink(target, &path).unwrap();
}

#[cfg(windows)]
fn take_lock(dir: &Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(FILE_SHARE_READ)
        .open(dir.join("lockfile"))
        .unwrap()
}

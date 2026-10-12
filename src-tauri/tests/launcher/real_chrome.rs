//! Lifecycle tests against a real Chromium (design §10). Linux only, and only on request:
//!
//! ```bash
//! FPL_TEST_CHROME=$HOME/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome \
//!   cargo test --test launcher -- --ignored
//! ```
//!
//! Chrome for Testing works: it is a complete multi-process Chromium that ignores the
//! `--fingerprint*` switches. It runs headless and sandboxed (no `--no-sandbox` needed on the
//! development VM).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fingerprint_launcher_lib::db::profiles::ProfileDraft;
use fingerprint_launcher_lib::error::ErrorCode;
use fingerprint_launcher_lib::kernel::lock::{self, LockState};
use fingerprint_launcher_lib::services::launcher::ProfileStatus;

use crate::support::{Fixture, block_on, config, eventually, kill_external, wait_group_gone};

use ProfileStatus::{Running, Starting, Stopped, Stopping};

const CHROME_ARGS: &str = "--headless=new --no-first-run --no-default-browser-check";

fn chrome() -> PathBuf {
    std::env::var_os("FPL_TEST_CHROME")
        .map(PathBuf::from)
        .expect("set FPL_TEST_CHROME to a Chromium executable")
}

/// Production time limits: the real browser must meet them.
fn fixture() -> Fixture {
    let f = Fixture::new(config(3000, 10_000, 5000));
    f.set_kernel(&chrome(), 151);
    f
}

fn chrome_profile(f: &Fixture, name: &str) -> String {
    f.profile_with(
        ProfileDraft {
            name: name.into(),
            fingerprint_seed: Some(1000),
            timezone: Some("America/Los_Angeles".into()),
            extra_args: Some(CHROME_ARGS.into()),
            start_urls: vec!["about:blank".into()],
            ..ProfileDraft::default()
        },
        "",
    )
}

/// Pids of processes whose command line mentions `dir`; Chromium passes the user data
/// directory to every child it starts.
fn processes_using(dir: &Path) -> Vec<u32> {
    let needle = dir.to_string_lossy().into_owned();
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
            String::from_utf8_lossy(&cmdline)
                .contains(&needle)
                .then_some(pid)
        })
        .collect()
}

/// Start, run as a process tree, stop gracefully, leave nothing behind (A5, A6).
pub fn real_chrome_starts_and_stops_without_remnants() {
    block_on(async {
        let f = fixture();
        let id = chrome_profile(&f, "p");
        let udd = f.udd(&id);
        let info = f.launcher.launch(&id).await.unwrap();
        f.wait_status(&id, Running).await;
        assert_eq!(
            lock::check(&udd),
            LockState::Held {
                pid: Some(info.pid)
            }
        );
        let tree = eventually("Chromium's child processes", || {
            let pids = processes_using(&udd);
            (pids.len() > 2).then_some(pids)
        })
        .await;
        assert!(tree.contains(&info.pid));

        let started = Instant::now();
        f.launcher.stop(&id).await;
        // Closed by SIGTERM, not by the kill at the end of the grace period.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(f.sink.statuses(&id), [Starting, Running, Stopping, Stopped]);
        wait_group_gone(info.pid).await;
        eventually("no Chromium process left", || {
            processes_using(&udd).is_empty().then_some(())
        })
        .await;
        assert_eq!(lock::check(&udd), LockState::Free);
    });
}

/// Two profiles run side by side on their own directories and stop independently (A9).
pub fn real_chrome_profiles_are_independent() {
    block_on(async {
        let f = fixture();
        let a = chrome_profile(&f, "a");
        let b = chrome_profile(&f, "b");
        let pid_a = f.launcher.launch(&a).await.unwrap().pid;
        let pid_b = f.launcher.launch(&b).await.unwrap().pid;
        f.wait_status(&a, Running).await;
        f.wait_status(&b, Running).await;
        assert_ne!(pid_a, pid_b);
        // Each directory has its own browser and its own profile data.
        assert_eq!(
            lock::check(&f.udd(&a)),
            LockState::Held { pid: Some(pid_a) }
        );
        assert_eq!(
            lock::check(&f.udd(&b)),
            LockState::Held { pid: Some(pid_b) }
        );
        for id in [&a, &b] {
            let profile_dir = f.udd(id).join("Default");
            eventually("the Default profile directory", || {
                profile_dir.is_dir().then_some(())
            })
            .await;
        }

        f.launcher.stop(&a).await;
        wait_group_gone(pid_a).await;
        // b is untouched.
        assert_eq!(f.sink.statuses(&b), [Starting, Running]);
        assert!(f.launcher.is_running(&b));
        assert!(processes_using(&f.udd(&b)).contains(&pid_b));

        f.launcher.stop(&b).await;
        wait_group_gone(pid_b).await;
        assert!(processes_using(&f.udd(&a)).is_empty());
        assert!(processes_using(&f.udd(&b)).is_empty());
    });
}

/// A Chromium the user started by hand on the profile's directory is detected through its
/// real `SingletonLock`; once it is killed, its stale lock does not block the launch.
pub fn real_chrome_singleton_lock_is_detected() {
    block_on(async {
        let f = fixture();
        let id = chrome_profile(&f, "p");
        let udd = f.udd(&id);
        let mut args = vec![format!("--user-data-dir={}", udd.display())];
        args.extend(CHROME_ARGS.split(' ').map(str::to_owned));
        let mut external = f.start_external_program(&chrome(), &args);
        eventually("the external Chromium's lock", || {
            matches!(lock::check(&udd), LockState::Held { .. }).then_some(())
        })
        .await;

        let err = f.launcher.launch(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileRunningExternally);
        assert!(
            err.message.contains(&external.id().to_string()),
            "{}",
            err.message
        );

        kill_external(&mut external);
        wait_group_gone(external.id()).await;
        // The symlink survives SIGKILL but names a dead process.
        assert!(udd.join("SingletonLock").symlink_metadata().is_ok());
        assert_eq!(lock::check(&udd), LockState::Free);

        let info = f.launcher.launch(&id).await.unwrap();
        f.wait_status(&id, Running).await;
        f.launcher.stop(&id).await;
        wait_group_gone(info.pid).await;
        assert_eq!(f.sink.statuses(&id).last(), Some(&Stopped));
    });
}

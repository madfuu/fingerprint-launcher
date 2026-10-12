//! Lifecycle tests against the fake kernel. They run on every OS unless marked otherwise.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use fingerprint_launcher_lib::db::profiles::{self, ProfileDraft};
use fingerprint_launcher_lib::error::ErrorCode;
use fingerprint_launcher_lib::kernel::probe;
use fingerprint_launcher_lib::services::launcher::{BatchProgress, FailureHint, ProfileStatus};

use crate::support::{
    Fixture, assert_child_dead, block_on, config, eventually, kill_external, wait_child_alive,
    wait_file,
};

use ProfileStatus::{Error, Running, Starting, Stopped, Stopping};

/// Start → Running with exactly the planned argv → stop → nothing left (A5, A6, R3.1).
pub fn launch_runs_the_planned_command_and_stops_cleanly() {
    block_on(async {
        let f = Fixture::new(config(300, 300, 1000));
        let id = f.profile("p", "");
        let udd = f.udd(&id);

        let info = f.launcher.launch(&id).await.unwrap();
        assert_eq!(info.status, Starting);
        assert_eq!(info.kernel_id, f.kernel.id);
        assert_eq!(info.user_data_dir, udd.to_string_lossy());
        wait_file(&udd.join("ready")).await;
        let argv: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(udd.join("argv.json")).unwrap()).unwrap();
        assert_eq!(argv, f.request(&id).plan.args);
        assert!(argv.contains(&"--fingerprint=1000".to_owned()), "{argv:?}");
        let main_pid: u32 = std::fs::read_to_string(udd.join("main.pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(main_pid, info.pid);

        let running = f.wait_status(&id, Running).await;
        assert_eq!(running.pid, Some(info.pid));
        let listed = f.launcher.list_running();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].status, listed[0].pid), (Running, info.pid));
        assert_eq!(f.launcher.running_ids(), std::slice::from_ref(&id));
        assert!(f.launcher.is_running(&id));
        let profile = profiles::get(&f.db.lock().unwrap(), &id).unwrap();
        assert!(profile.last_launched_at.is_some());
        wait_child_alive(&udd).await;

        f.launcher.stop(&id).await;
        assert_eq!(f.sink.statuses(&id), [Starting, Running, Stopping, Stopped]);
        assert!(f.sink.events_for(&id).iter().all(|e| e.error.is_none()));
        assert!(f.launcher.list_running().is_empty());
        assert!(!f.launcher.is_running(&id));
        assert_child_dead(&udd).await;
        #[cfg(unix)]
        crate::support::wait_group_gone(info.pid).await;
        let log = std::fs::read_to_string(f.log_file(&id)).unwrap();
        assert!(log.starts_with("=== "), "{log}");
        // Stopping again is a no-op.
        f.launcher.stop(&id).await;
    });
}

/// SIGTERM closes the browser well within the grace period; the group is empty afterwards.
#[cfg(unix)]
pub fn graceful_stop_does_not_wait_for_the_grace_period() {
    block_on(async {
        let f = Fixture::new(config(200, 200, 10_000));
        let id = f.profile("p", "");
        let info = f.launcher.launch(&id).await.unwrap();
        f.wait_status(&id, Running).await;
        wait_child_alive(&f.udd(&id)).await;

        let started = Instant::now();
        f.launcher.stop(&id).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(f.sink.statuses(&id).last(), Some(&Stopped));
        crate::support::wait_group_gone(info.pid).await;
        assert_child_dead(&f.udd(&id)).await;
    });
}

/// A browser that ignores SIGTERM is killed with its whole group once the grace period ends.
#[cfg(unix)]
pub fn sigterm_ignored_means_killed_after_the_grace_period() {
    block_on(async {
        let f = Fixture::new(config(200, 200, 800));
        let id = f.profile("p", "ignore-term");
        let info = f.launcher.launch(&id).await.unwrap();
        f.wait_status(&id, Running).await;
        wait_child_alive(&f.udd(&id)).await;

        let started = Instant::now();
        f.launcher.stop(&id).await;
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(800), "{took:?}");
        assert!(took < Duration::from_secs(10), "{took:?}");
        assert_eq!(f.sink.statuses(&id), [Starting, Running, Stopping, Stopped]);
        crate::support::wait_group_gone(info.pid).await;
        assert_child_dead(&f.udd(&id)).await;
    });
}

/// The browser goes away without the launcher: killed from outside, or closed by the user.
/// The launcher notices within seconds (A5) and sweeps up the orphaned child.
pub fn browser_exiting_on_its_own_is_noticed() {
    block_on(async {
        let f = Fixture::new(config(300, 300, 1000));
        let killed = f.profile("killed", "");
        let closed = f.profile("closed", "exit-after 1500\nexit-code 0");
        let info = f.launcher.launch(&killed).await.unwrap();
        f.launcher.launch(&closed).await.unwrap();
        f.wait_status(&killed, Running).await;
        wait_child_alive(&f.udd(&killed)).await;

        kill_main_process(info.pid);
        let killed_at = Instant::now();
        let stopped = f.wait_status(&killed, Stopped).await;
        assert!(killed_at.elapsed() < Duration::from_secs(5));
        assert_eq!(stopped.error, None);
        assert_eq!(f.sink.statuses(&killed), [Starting, Running, Stopped]);
        assert_child_dead(&f.udd(&killed)).await;
        #[cfg(unix)]
        crate::support::wait_group_gone(info.pid).await;

        // A clean exit after the startup window is a normal stop.
        let stopped = f.wait_status(&closed, Stopped).await;
        assert_eq!(stopped.error, None);
        assert_eq!(f.sink.statuses(&closed), [Starting, Running, Stopped]);
        assert_child_dead(&f.udd(&closed)).await;
        assert!(f.launcher.list_running().is_empty());
    });
}

/// Kills only the main process, as a crash or the task manager would.
fn kill_main_process(pid: u32) {
    #[cfg(unix)]
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();
    #[cfg(windows)]
    assert!(
        std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}

/// Exiting during startup is an error carrying the stderr tail and a hint (R3.6, A12).
pub fn early_exit_is_an_error_with_stderr_and_hint() {
    block_on(async {
        // The fakes exit at once; long windows keep a slow runner from seeing `Running` first.
        let f = Fixture::new(config(5000, 10_000, 1000));
        let sandbox = f.profile(
            "sandbox",
            "exit-after 0\nexit-code 3\nstderr first line\n\
             stderr [1:1:FATAL:zygote_host_impl_linux.cc(128)] No usable sandbox! Update your kernel\n\
             stderr last line",
        );
        let fuse = f.profile(
            "fuse",
            "exit-after 0\nexit-code 127\nstderr dlopen(): error loading libfuse.so.2",
        );
        f.launcher.launch(&sandbox).await.unwrap();
        f.launcher.launch(&fuse).await.unwrap();

        let event = f.wait_status(&sandbox, Error).await;
        assert_eq!(f.sink.statuses(&sandbox), [Starting, Error]);
        let error = event.error.unwrap();
        assert_eq!(error.code, ErrorCode::StartupFailed);
        assert_eq!(error.exit_code, Some(3));
        assert_eq!(error.hint, Some(FailureHint::NoSandbox));
        assert_eq!(error.stderr_tail.first().unwrap(), "first line");
        assert_eq!(error.stderr_tail.last().unwrap(), "last line");
        assert!(error.message.contains("last line"), "{}", error.message);
        let log = std::fs::read_to_string(f.log_file(&sandbox)).unwrap();
        assert!(log.contains("No usable sandbox"), "{log}");

        let error = f.wait_status(&fuse, Error).await.error.unwrap();
        assert_eq!(error.exit_code, Some(127));
        assert_eq!(error.hint, Some(FailureHint::MissingFuse));
        assert!(f.launcher.list_running().is_empty());
    });
}

/// A failing exit soon after `Running` still counts as a failed start; a clean one does not.
pub fn failure_inside_the_early_exit_window_is_an_error() {
    block_on(async {
        // `Running` after 200 ms, exit after 1.5 s, both well inside the 10 s window: the margins
        // absorb a slow runner.
        let f = Fixture::new(config(200, 10_000, 1000));
        let failing = f.profile("failing", "exit-after 1500\nexit-code 1\nstderr crashed");
        let clean = f.profile("clean", "exit-after 1500\nexit-code 0");
        f.launcher.launch(&failing).await.unwrap();
        f.launcher.launch(&clean).await.unwrap();

        let error = f.wait_status(&failing, Error).await.error.unwrap();
        assert_eq!(f.sink.statuses(&failing), [Starting, Running, Error]);
        assert_eq!(error.code, ErrorCode::StartupFailed);
        assert_eq!(error.exit_code, Some(1));
        assert_eq!(error.stderr_tail, ["crashed"]);
        assert_eq!(error.hint, None);

        f.wait_status(&clean, Stopped).await;
        assert_eq!(f.sink.statuses(&clean), [Starting, Running, Stopped]);
    });
}

/// Kernel problems fail before any spawn, with a readable error and an `Error` event (A12).
pub fn kernel_problems_fail_before_spawning() {
    block_on(async {
        let f = Fixture::new(config(300, 300, 1000));
        let id = f.profile("p", "");
        let launch = |expected: ErrorCode| {
            let f = &f;
            let id = id.clone();
            async move {
                let before = f.sink.events_for(&id).len();
                let err = f.launcher.launch(&id).await.unwrap_err();
                assert_eq!(err.code, expected, "{}", err.message);
                let events = f.sink.events_for(&id);
                assert_eq!(events.len(), before + 1);
                let event = events.last().unwrap();
                assert_eq!(event.status, Error);
                assert_eq!(event.error.as_ref().unwrap().code, expected);
                assert!(!f.launcher.is_running(&id));
            }
        };

        f.set_kernel(&f.path("missing-chrome"), 150);
        launch(ErrorCode::KernelMissing).await;

        std::fs::create_dir(f.path("a-directory")).unwrap();
        f.set_kernel(&f.path("a-directory"), 150);
        launch(ErrorCode::KernelNotExecutable).await;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let plain = f.path("chrome");
            std::fs::write(&plain, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
            f.set_kernel(&plain, 150);
            launch(ErrorCode::KernelNotExecutable).await;
            // Executable, but the OS cannot run it: its interpreter does not exist.
            let broken = f.path("broken-chrome");
            std::fs::write(&broken, "#!/nonexistent/interpreter\n").unwrap();
            std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
            f.set_kernel(&broken, 150);
            launch(ErrorCode::SpawnFailed).await;
        }
        #[cfg(windows)]
        {
            let broken = f.path("broken-chrome.exe");
            std::fs::write(&broken, b"not a program").unwrap();
            f.set_kernel(&broken, 150);
            launch(ErrorCode::SpawnFailed).await;
        }

        // No kernel at all.
        fingerprint_launcher_lib::db::kernels::delete(&f.db.lock().unwrap(), &f.kernel.id).unwrap();
        launch(ErrorCode::NoKernel).await;
        // And the launcher still works afterwards.
        assert!(f.launcher.list_running().is_empty());
    });
}

/// A profile cannot run twice, not even with two simultaneous launches (R3.4).
pub fn duplicate_launches_are_rejected() {
    block_on(async {
        let f = Fixture::new(config(300, 300, 1000));
        let id = f.profile("p", "");
        let (a, b) = tokio::join!(f.launcher.launch(&id), f.launcher.launch(&id));
        let (ok, err) = match (a, b) {
            (Ok(info), Err(err)) | (Err(err), Ok(info)) => (info, err),
            other => panic!("expected one launch to win: {other:?}"),
        };
        assert_eq!(err.code, ErrorCode::ProfileRunning);
        f.wait_status(&id, Running).await;
        let again = f.launcher.launch(&id).await.unwrap_err();
        assert_eq!(again.code, ErrorCode::ProfileRunning);
        // Rejections leave the running profile's status alone.
        assert_eq!(f.sink.statuses(&id), [Starting, Running]);
        assert_eq!(f.launcher.list_running()[0].pid, ok.pid);

        f.launcher.stop(&id).await;
        assert_eq!(f.sink.statuses(&id), [Starting, Running, Stopping, Stopped]);
    });
}

/// A browser started by hand on the profile's directory blocks the launch; once it is gone,
/// its stale lock does not (design §4.2 step 2).
pub fn external_browser_blocks_the_launch() {
    block_on(async {
        let f = Fixture::new(config(300, 300, 1000));
        let id = f.profile("p", "lock");
        let udd = f.udd(&id);
        let mut external = f.start_external(&udd);
        wait_file(&udd.join("ready")).await;

        let err = f.launcher.launch(&id).await.unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::ProfileRunningExternally,
            "{}",
            err.message
        );
        assert_eq!(f.sink.statuses(&id), [Error]);
        #[cfg(unix)]
        assert!(
            err.message.contains(&external.id().to_string()),
            "{}",
            err.message
        );

        kill_external(&mut external);
        // Unix: the symlink is still there but names a dead process. Windows: the file is no
        // longer held.
        f.launcher.launch(&id).await.unwrap();
        f.wait_status(&id, Running).await;
        f.launcher.stop(&id).await;
        assert_eq!(f.sink.statuses(&id).last(), Some(&Stopped));
    });
}

/// Two profiles sharing a custom directory cannot run at the same time.
pub fn shared_user_data_dir_is_a_conflict() {
    block_on(async {
        let f = Fixture::new(config(300, 300, 1000));
        let shared = f.path("shared-dir");
        let draft = |name: &str| ProfileDraft {
            name: name.into(),
            user_data_dir: Some(shared.to_str().unwrap().into()),
            ..ProfileDraft::default()
        };
        let a = f.profile_with(draft("a"), "");
        let b = f.profile_with(draft("b"), "");
        f.launcher.launch(&a).await.unwrap();
        let err = f.launcher.launch(&b).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Conflict, "{}", err.message);
        assert!(err.message.contains(&a), "{}", err.message);
        f.launcher.stop(&a).await;
    });
}

/// Batch launch keeps at most `concurrency` profiles in `Starting` and waits `interval`
/// before each spawn (R4.2, A8).
pub fn batch_launch_limits_concurrency_and_spaces_spawns() {
    block_on(async {
        // The second spawn must come before the first profile is `Running` for two to be
        // starting at once: 1 s of startup leaves ample room for the 150 ms interval plus the
        // database round trips of a slow runner.
        let f = Fixture::new(config(1000, 1000, 1000));
        let ids: Vec<String> = (0..6).map(|i| f.profile(&format!("p{i}"), "")).collect();
        let interval = Duration::from_millis(150);

        let progress = f.launcher.batch_launch(&ids, 2, interval).await;
        assert_eq!(
            progress,
            BatchProgress {
                total: 6,
                launched: 6,
                failed: 0,
                skipped: 0
            }
        );
        let reported: Vec<usize> = f.sink.progress().iter().map(|p| p.launched).collect();
        assert_eq!(reported, [1, 2, 3, 4, 5, 6]);
        for id in &ids {
            f.wait_status(id, Running).await;
        }

        // Replay the events: how many profiles were in Starting at each moment.
        let mut last: HashMap<String, ProfileStatus> = HashMap::new();
        let (mut starting, mut max_starting) = (0, 0);
        let mut spawn_times = Vec::new();
        for (at, event) in f.sink.events() {
            if event.status == Starting {
                starting += 1;
                spawn_times.push(at);
            } else if last.get(&event.profile_id) == Some(&Starting) {
                starting -= 1;
            }
            last.insert(event.profile_id, event.status);
            max_starting = max_starting.max(starting);
        }
        assert_eq!(
            max_starting, 2,
            "never more (or fewer) than 2 starting at once"
        );
        for pair in spawn_times.windows(2) {
            assert!(pair[1] - pair[0] >= interval, "{:?}", pair[1] - pair[0]);
        }

        f.launcher.stop_many(&ids).await;
        for id in &ids {
            assert_eq!(f.sink.statuses(id), [Starting, Running, Stopping, Stopped]);
            assert_child_dead(&f.udd(id)).await;
        }
        assert!(f.launcher.list_running().is_empty());
    });
}

/// A failure or a duplicate does not end the batch.
pub fn batch_launch_continues_after_failures() {
    block_on(async {
        let f = Fixture::new(config(300, 300, 1000));
        let a = f.profile("a", "");
        let b = f.profile("b", "");
        let ids = [a.clone(), "missing".to_owned(), a.clone(), b.clone()];

        let progress = f.launcher.batch_launch(&ids, 1, Duration::ZERO).await;
        assert_eq!(
            progress,
            BatchProgress {
                total: 4,
                launched: 2,
                failed: 1,
                skipped: 1
            }
        );
        let missing = f.sink.events_for("missing");
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].error.as_ref().unwrap().code, ErrorCode::NotFound);
        f.launcher.stop_many(&[a, b]).await;
        assert!(f.launcher.list_running().is_empty());
    });
}

/// Stopping during startup is a stop, not a failed start.
pub fn stop_during_startup_is_a_stop() {
    block_on(async {
        let f = Fixture::new(config(5000, 5000, 1000));
        let id = f.profile("p", "");
        f.launcher.launch(&id).await.unwrap();
        f.launcher.stop(&id).await;
        assert_eq!(f.sink.statuses(&id), [Starting, Stopping, Stopped]);
        assert!(f.sink.events_for(&id).iter().all(|e| e.error.is_none()));
    });
}

/// The quit path stops everything (A7), including a browser that ignores SIGTERM, and refuses
/// new launches.
pub fn shutdown_stops_everything() {
    block_on(async {
        let f = Fixture::new(config(200, 200, 600));
        let ids = [
            f.profile("a", ""),
            f.profile("b", "ignore-term"),
            f.profile("c", "no-child"),
        ];
        let mut pids = Vec::new();
        for id in &ids {
            pids.push(f.launcher.launch(id).await.unwrap().pid);
        }
        for id in &ids {
            f.wait_status(id, Running).await;
        }

        f.launcher.shutdown().await;
        assert!(f.launcher.list_running().is_empty());
        assert!(f.launcher.running_ids().is_empty());
        for id in &ids {
            assert_eq!(f.sink.statuses(id).last(), Some(&Stopped));
        }
        assert_child_dead(&f.udd(&ids[0])).await;
        assert_child_dead(&f.udd(&ids[1])).await;
        #[cfg(unix)]
        for pid in pids {
            crate::support::wait_group_gone(pid).await;
        }

        let err = f.launcher.launch(&ids[0]).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ShuttingDown);
        assert_eq!(f.sink.statuses(&ids[0]).last(), Some(&Stopped));
        // A batch during shutdown skips everything.
        let progress = f.launcher.batch_launch(&ids, 2, Duration::ZERO).await;
        assert_eq!((progress.launched, progress.skipped), (0, 3));
    });
}

/// stderr is read while the browser runs: 1 MiB (16 times a pipe buffer) is written before
/// the fake reports ready, and all of it reaches the log.
pub fn stderr_is_drained_and_logged() {
    block_on(async {
        let f = Fixture::new(config(200, 200, 1000));
        let id = f.profile("p", "stderr-flood 1024");
        f.launcher.launch(&id).await.unwrap();
        wait_file(&f.udd(&id).join("ready")).await;
        let log = f.log_file(&id);
        eventually("the whole flood in the log", || {
            let len = std::fs::metadata(&log).ok()?.len();
            (len >= 1024 * 1024).then_some(())
        })
        .await;
        f.launcher.stop(&id).await;
        assert_eq!(f.sink.statuses(&id).last(), Some(&Stopped));
    });
}

/// `--version` probing runs the kernel; on Windows a PE is never run.
pub fn probe_reads_the_version_flag() {
    block_on(async {
        let exe = std::env::current_exe().unwrap();
        let probe = probe::probe_kernel(&exe, Duration::from_secs(10)).await;
        #[cfg(unix)]
        {
            assert_eq!(probe.method, Some(probe::ProbeMethod::VersionFlag));
            assert_eq!(probe.major_version, Some(150));
            assert_eq!(probe.full_version.as_deref(), Some("150.0.7871.186"));
        }
        #[cfg(windows)]
        {
            // This test binary has no version resource, and is not executed to find one.
            assert_eq!(probe.method, None);
        }
    });
}

//! Process lifecycle tests for `services::launcher` (design §10).
//!
//! This binary is also the fake kernel the tests launch (`fake.rs`): `main` first checks
//! whether it was started as a browser, and only otherwise runs the tests. That is why it has
//! no libtest harness (`harness = false` in Cargo.toml) and brings a small runner of its own
//! (`harness.rs`). The fake is the same file on every OS, and no extra build artifact is needed.
//!
//! `cargo test` runs the fake-kernel tests; `cargo test -- --ignored` runs the real-Chromium
//! tests (Linux, `FPL_TEST_CHROME`).

mod cases;
mod fake;
mod harness;
#[cfg(target_os = "linux")]
mod real_chrome;
mod support;

use std::process::ExitCode;

use harness::Test;

macro_rules! tests {
    ($($(#[$cfg:meta])* $module:ident::$name:ident $(($ignored:ident))?),+ $(,)?) => {
        &[$(
            $(#[$cfg])*
            Test {
                name: stringify!($name),
                run: $module::$name,
                ignored: tests!(@ignored $($ignored)?),
            },
        )+]
    };
    (@ignored ignored) => { true };
    (@ignored) => { false };
}

const TESTS: &[Test] = tests![
    cases::launch_runs_the_planned_command_and_stops_cleanly,
    #[cfg(unix)]
    cases::graceful_stop_does_not_wait_for_the_grace_period,
    #[cfg(unix)]
    cases::sigterm_ignored_means_killed_after_the_grace_period,
    cases::browser_exiting_on_its_own_is_noticed,
    cases::early_exit_is_an_error_with_stderr_and_hint,
    cases::failure_inside_the_early_exit_window_is_an_error,
    cases::kernel_problems_fail_before_spawning,
    cases::duplicate_launches_are_rejected,
    cases::external_browser_blocks_the_launch,
    cases::shared_user_data_dir_is_a_conflict,
    cases::batch_launch_limits_concurrency_and_spaces_spawns,
    cases::batch_launch_continues_after_failures,
    cases::stop_during_startup_is_a_stop,
    cases::shutdown_stops_everything,
    cases::stderr_is_drained_and_logged,
    cases::probe_reads_the_version_flag,
    cases::running_profiles_cannot_be_deleted,
    #[cfg(target_os = "linux")]
    real_chrome::real_chrome_starts_and_stops_without_remnants(ignored),
    #[cfg(target_os = "linux")]
    real_chrome::real_chrome_profiles_are_independent(ignored),
    #[cfg(target_os = "linux")]
    real_chrome::real_chrome_singleton_lock_is_detected(ignored),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    fake::dispatch(&args);
    harness::run(TESTS, &args)
}

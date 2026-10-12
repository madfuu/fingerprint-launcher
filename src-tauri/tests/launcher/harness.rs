//! A small stand-in for the libtest runner (this binary has its own `main`): runs every
//! selected test on its own thread and prints libtest-style results.
//!
//! Understands what `cargo test` passes through: name filters (substring, or exact with
//! `--exact`), `--skip <filter>`, `--ignored`, `--include-ignored` and `--list`. Other flags are
//! accepted and ignored.
//!
//! Unlike libtest it has a watchdog: a test that hangs (e.g. a stop that never completes) fails
//! the run after [`RUN_TIMEOUT`] instead of holding a CI runner until the job times out.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Upper bound for the whole run. The tests take seconds; this only catches hangs.
const RUN_TIMEOUT: Duration = Duration::from_secs(300);

pub struct Test {
    pub name: &'static str,
    pub run: fn(),
    /// Only run with `--ignored` / `--include-ignored` (e.g. needs a real Chromium).
    pub ignored: bool,
}

/// Flags of libtest that take a separate value.
const VALUE_FLAGS: &[&str] = &[
    "--skip",
    "--test-threads",
    "--format",
    "--color",
    "--logfile",
    "--shuffle-seed",
    "-Z",
];

pub fn run(tests: &[Test], args: &[String]) -> ExitCode {
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let (mut exact, mut ignored_only, mut include_ignored, mut list) = (false, false, false, false);
    let mut rest = args.iter().skip(1);
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--exact" => exact = true,
            "--ignored" => ignored_only = true,
            "--include-ignored" => include_ignored = true,
            "--list" => list = true,
            "--skip" => skips.extend(rest.next().cloned()),
            flag if VALUE_FLAGS.contains(&flag) => {
                rest.next();
            }
            flag if flag.starts_with('-') => {}
            filter => filters.push(filter.to_owned()),
        }
    }
    let matches = |name: &str, pattern: &str| {
        if exact {
            name == pattern
        } else {
            name.contains(pattern)
        }
    };
    let selected: Vec<&Test> = tests
        .iter()
        .filter(|t| filters.is_empty() || filters.iter().any(|f| matches(t.name, f)))
        .filter(|t| !skips.iter().any(|s| matches(t.name, s)))
        .collect();
    let filtered_out = tests.len() - selected.len();

    if list {
        for test in &selected {
            println!("{}: test", test.name);
        }
        return ExitCode::SUCCESS;
    }

    let (to_run, ignored): (Vec<&Test>, Vec<&Test>) = selected
        .into_iter()
        .partition(|t| include_ignored || t.ignored == ignored_only);
    println!("\nrunning {} tests", to_run.len());

    let started = Instant::now();
    let (results_tx, results_rx) = mpsc::channel();
    for test in &to_run {
        let results_tx = results_tx.clone();
        let (name, run) = (test.name, test.run);
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let passed = catch_unwind(AssertUnwindSafe(run)).is_ok();
                let _ = results_tx.send((name, passed));
            })
            .expect("spawn test thread");
    }
    drop(results_tx);

    let mut failed = Vec::new();
    let mut pending: Vec<&str> = to_run.iter().map(|t| t.name).collect();
    let deadline = started + RUN_TIMEOUT;
    while !pending.is_empty() {
        match results_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((name, passed)) => {
                pending.retain(|&p| p != name);
                if passed {
                    println!("test {name} ... ok");
                } else {
                    println!("test {name} ... FAILED");
                    failed.push(name);
                }
            }
            // Returning from `main` ends the process, hung threads included.
            Err(RecvTimeoutError::Timeout) => {
                for name in pending.drain(..) {
                    println!(
                        "test {name} ... FAILED (still running after {}s)",
                        RUN_TIMEOUT.as_secs()
                    );
                    failed.push(name);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let passed = to_run.len() - failed.len();
    if !failed.is_empty() {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
    }
    println!(
        "\ntest result: {}. {passed} passed; {} failed; {} ignored; 0 measured; \
         {filtered_out} filtered out; finished in {:.2}s\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        failed.len(),
        ignored.len(),
        started.elapsed().as_secs_f64(),
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

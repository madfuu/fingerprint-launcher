//! Test fixture: a database with one kernel (the fake by default), a launcher wired to it, and a
//! sink that records every event; plus polling helpers and process checks.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fingerprint_launcher_lib::db::kernels::{self, Kernel, KernelInput};
use fingerprint_launcher_lib::db::profiles::{self, ProfileDraft};
use fingerprint_launcher_lib::db::{self, DB_FILE_NAME};
use fingerprint_launcher_lib::services::launcher::{
    BatchProgress, DbLaunchSource, LaunchRequest, LaunchSource, Launcher, LauncherConfig,
    ProfileStatus, StatusEvent, StatusSink,
};
use rusqlite::Connection;
use tempfile::TempDir;

use crate::fake::CONFIG_FILE;

/// Upper bound for anything a test waits for; generous for slow CI runners.
pub const DEADLINE: Duration = Duration::from_secs(20);

pub fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// Short time limits for tests; override fields as needed.
pub fn config(startup_ms: u64, early_exit_ms: u64, stop_grace_ms: u64) -> LauncherConfig {
    LauncherConfig {
        startup_grace: Duration::from_millis(startup_ms),
        early_exit_window: Duration::from_millis(early_exit_ms),
        stop_grace: Duration::from_millis(stop_grace_ms),
        log_dir: None,
    }
}

/// Polls `probe` every 10 ms until it yields a value; panics after [`DEADLINE`].
pub async fn eventually<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<(Instant, StatusEvent)>>,
    progress: Mutex<Vec<BatchProgress>>,
}

impl StatusSink for RecordingSink {
    fn status_changed(&self, event: StatusEvent) {
        self.events.lock().unwrap().push((Instant::now(), event));
    }

    fn batch_progress(&self, progress: BatchProgress) {
        self.progress.lock().unwrap().push(progress);
    }
}

impl RecordingSink {
    pub fn events(&self) -> Vec<(Instant, StatusEvent)> {
        self.events.lock().unwrap().clone()
    }

    pub fn events_for(&self, id: &str) -> Vec<StatusEvent> {
        self.events()
            .into_iter()
            .map(|(_, event)| event)
            .filter(|event| event.profile_id == id)
            .collect()
    }

    pub fn statuses(&self, id: &str) -> Vec<ProfileStatus> {
        self.events_for(id)
            .iter()
            .map(|event| event.status)
            .collect()
    }

    pub fn progress(&self) -> Vec<BatchProgress> {
        self.progress.lock().unwrap().clone()
    }

    fn pids(&self) -> Vec<u32> {
        self.events().iter().filter_map(|(_, e)| e.pid).collect()
    }
}

pub struct Fixture {
    pub launcher: Launcher,
    pub sink: Arc<RecordingSink>,
    pub source: Arc<DbLaunchSource>,
    pub db: Arc<Mutex<Connection>>,
    pub kernel: Kernel,
    /// Pids of fakes started outside the launcher, killed if the test panics.
    external: Mutex<Vec<u32>>,
    root: PathBuf,
    /// Last: removed after everything above has released its files.
    pub dir: TempDir,
}

impl Fixture {
    /// The fake kernel (this binary) as the default kernel, major version 150.
    pub fn new(mut config: LauncherConfig) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join(DB_FILE_NAME)).unwrap();
        let exe = std::env::current_exe().unwrap();
        let kernel = kernels::create(
            &conn,
            KernelInput {
                name: "fake".into(),
                exec_path: exe.to_str().unwrap().into(),
                major_version: 150,
                full_version: None,
            },
        )
        .unwrap();
        let db = Arc::new(Mutex::new(conn));
        let root = dir.path().join("profiles");
        config.log_dir = Some(dir.path().join("logs"));
        let source = Arc::new(DbLaunchSource::new(Arc::clone(&db), root.clone()));
        let sink = Arc::new(RecordingSink::default());
        let launcher = Launcher::new(config, source.clone(), sink.clone());
        Self {
            launcher,
            sink,
            source,
            db,
            kernel,
            external: Mutex::new(Vec::new()),
            root,
            dir,
        }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Points the kernel at another executable.
    pub fn set_kernel(&self, exec_path: &Path, major_version: u32) {
        kernels::update(
            &self.db.lock().unwrap(),
            &self.kernel.id,
            KernelInput {
                name: "kernel".into(),
                exec_path: exec_path.to_str().unwrap().into(),
                major_version,
                full_version: None,
            },
        )
        .unwrap();
    }

    /// A profile with fingerprint arguments, configured for the fake by `fake_cfg`.
    pub fn profile(&self, name: &str, fake_cfg: &str) -> String {
        self.profile_with(
            ProfileDraft {
                name: name.into(),
                fingerprint_seed: Some(1000),
                lang: Some("zh-CN".into()),
                timezone: Some("America/Los_Angeles".into()),
                ..ProfileDraft::default()
            },
            fake_cfg,
        )
    }

    pub fn profile_with(&self, draft: ProfileDraft, fake_cfg: &str) -> String {
        let profile = profiles::create(&self.db.lock().unwrap(), draft).unwrap();
        let udd = self.request(&profile.id).user_data_dir;
        std::fs::create_dir_all(&udd).unwrap();
        std::fs::write(udd.join(CONFIG_FILE), fake_cfg).unwrap();
        profile.id
    }

    /// The default user data directory of a profile.
    pub fn udd(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn request(&self, id: &str) -> LaunchRequest {
        self.source.resolve(id).unwrap()
    }

    pub fn log_file(&self, id: &str) -> PathBuf {
        self.dir.path().join("logs").join(format!("{id}.log"))
    }

    pub async fn wait_status(&self, id: &str, status: ProfileStatus) -> StatusEvent {
        eventually(&format!("{id} to be {status:?}"), || {
            self.sink
                .events_for(id)
                .into_iter()
                .find(|event| event.status == status)
        })
        .await
    }

    /// Starts the fake on `udd` without the launcher, as a browser the user opened by hand.
    pub fn start_external(&self, udd: &Path) -> Child {
        let exe = std::env::current_exe().unwrap();
        self.start_external_program(&exe, &[format!("--user-data-dir={}", udd.display())])
    }

    /// Starts `program` outside the launcher (its own process group on Unix).
    pub fn start_external_program(&self, program: &Path, args: &[String]) -> Child {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let child = command.spawn().unwrap();
        self.external.lock().unwrap().push(child.id());
        child
    }
}

impl Drop for Fixture {
    /// A failed assertion must not leak processes: kill every tree this test started. (On
    /// Windows the launcher's job objects also do this when the runtime drops its tasks.)
    fn drop(&mut self) {
        if std::thread::panicking() {
            let external = self.external.lock().map(|p| p.clone()).unwrap_or_default();
            for pid in self.sink.pids().into_iter().chain(external) {
                kill_tree(pid);
            }
        }
    }
}

/// Kills the process tree rooted at `pid` (a process group leader on Unix).
pub fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{Signal, killpg};
        let _ = killpg(nix::unistd::Pid::from_raw(pid as i32), Signal::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Kills an external fake and reaps it.
pub fn kill_external(child: &mut Child) {
    kill_tree(child.id());
    child.wait().unwrap();
}

pub async fn wait_file(path: &Path) {
    eventually(&format!("{}", path.display()), || {
        path.exists().then_some(())
    })
    .await;
}

fn beat(udd: &Path) -> Option<String> {
    std::fs::read_to_string(udd.join("child.beat")).ok()
}

/// Waits until the fake's child is alive (its heartbeat changes).
pub async fn wait_child_alive(udd: &Path) {
    let first = eventually("the child's first heartbeat", || beat(udd)).await;
    eventually("the child's heartbeat to change", || {
        beat(udd).filter(|now| *now != first).map(drop)
    })
    .await;
}

/// The fake's child is dead: its heartbeat stands still for longer than a dozen beats.
pub async fn assert_child_dead(udd: &Path) {
    let before = beat(udd);
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        beat(udd),
        before,
        "the child of {} is still alive",
        udd.display()
    );
}

/// Unix: the process group is empty (every member exited and was reaped). Killed members can
/// linger as zombies for a moment, hence the wait.
#[cfg(unix)]
pub async fn wait_group_gone(pgid: u32) {
    use nix::errno::Errno;
    use nix::sys::signal::killpg;
    let pgid = nix::unistd::Pid::from_raw(pgid as i32);
    eventually(&format!("process group {pgid} to be empty"), || {
        (killpg(pgid, None) == Err(Errno::ESRCH)).then_some(())
    })
    .await;
}

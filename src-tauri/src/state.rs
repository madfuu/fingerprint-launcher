//! State shared by every command: the database, the launcher and the app's file locations.
//! Plain Rust, so services can be tested without a Tauri runtime.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rusqlite::Connection;

use crate::db;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::paths::AppPaths;
use crate::services::launcher::{DbLaunchSource, Launcher, LauncherConfig, StatusSink};

pub struct AppState {
    /// One connection for the whole app (single-user desktop, design §4.1). Only locked on the
    /// blocking pool, through [`AppState::db`].
    db: Arc<Mutex<Connection>>,
    pub launcher: Launcher,
    pub paths: AppPaths,
}

impl AppState {
    /// Opens (creating and migrating) the database and builds the launcher on it.
    pub fn open(paths: AppPaths, sink: Arc<dyn StatusSink>) -> AppResult<Self> {
        Self::with_config(
            paths.clone(),
            sink,
            LauncherConfig {
                log_dir: Some(paths.log_dir()),
                ..LauncherConfig::default()
            },
        )
    }

    /// [`AppState::open`] with explicit launcher time limits (tests).
    pub fn with_config(
        paths: AppPaths,
        sink: Arc<dyn StatusSink>,
        config: LauncherConfig,
    ) -> AppResult<Self> {
        let db = Arc::new(Mutex::new(db::open(&paths.db_file())?));
        let source = DbLaunchSource::new(Arc::clone(&db), paths.default_user_data_root());
        Ok(Self {
            launcher: Launcher::new(config, Arc::new(source), sink),
            db,
            paths,
        })
    }

    /// Runs `f` with the connection on the blocking thread pool, so neither the async runtime
    /// nor the UI thread waits on SQLite or the lock.
    pub async fn db<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> AppResult<T> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || f(&lock(&db)))
            .await
            .unwrap_or_else(|err| Err(AppError::new(ErrorCode::Db, err.to_string())))
    }
}

fn lock(db: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    // A panic while the lock was held cannot leave SQLite inconsistent (transactions roll back
    // on drop), so a poisoned lock is still usable.
    db.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::AppState;
    use crate::paths::AppPaths;
    use crate::services::launcher::{BatchProgress, StatusEvent, StatusSink};

    /// Discards launcher events.
    pub(crate) struct NullSink;

    impl StatusSink for NullSink {
        fn status_changed(&self, _: StatusEvent) {}
        fn batch_progress(&self, _: BatchProgress) {}
    }

    /// An [`AppState`] whose data directory is a temp dir.
    pub(crate) struct TestState {
        // Dropped first: the connection must be closed before the directory is removed.
        pub state: AppState,
        pub dir: TempDir,
    }

    impl TestState {
        pub fn new() -> Self {
            let dir = tempfile::tempdir().expect("create temp dir");
            let paths = AppPaths::new(dir.path().join("data"));
            let state = AppState::open(paths, Arc::new(NullSink)).expect("open state");
            Self { state, dir }
        }
    }
}

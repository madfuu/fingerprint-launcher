//! Where launches come from: the profile, its kernel and its user data directory, resolved into
//! a [`LaunchRequest`]. The launcher only sees the [`LaunchSource`] trait, so its lifecycle
//! tests need neither Tauri nor (when they bring their own source) a database.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rusqlite::Connection;

use crate::db::profiles::Profile;
use crate::db::{kernels, profiles, proxies, settings};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::kernel::args::{LaunchPlan, build_launch_plan};
use crate::kernel::shell::HostPlatform;

/// What the launcher needs to start one profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub kernel_id: String,
    pub plan: LaunchPlan,
    /// The directory the plan passes as `--user-data-dir`. Checked for a browser running
    /// outside the launcher, and created, before the spawn.
    pub user_data_dir: PathBuf,
}

/// Supplies launch requests and records launches. Both methods block (database access); the
/// launcher calls them on the blocking thread pool.
pub trait LaunchSource: Send + Sync + 'static {
    /// The request for `profile_id`, or why it cannot be launched (`not_found`, `no_kernel`,
    /// `invalid_input`, ...).
    fn resolve(&self, profile_id: &str) -> AppResult<LaunchRequest>;

    /// Called once the browser process exists (design §4.2 step 6). A failure here does not
    /// undo the launch.
    fn launched(&self, profile_id: &str) -> AppResult<()>;
}

/// The user data directory of a stored profile: its own setting, or `<root>/<profile id>`.
pub fn user_data_dir(profile: &Profile, root: &Path) -> PathBuf {
    profile
        .draft
        .user_data_dir
        .as_deref()
        .map_or_else(|| root.join(&profile.id), PathBuf::from)
}

/// [`LaunchSource`] over the application database.
pub struct DbLaunchSource {
    db: Arc<Mutex<Connection>>,
    /// Root of the default user data directories when the `user_data_root` setting is unset
    /// (`<app_data_dir>/profiles`, which only the caller knows).
    default_user_data_root: PathBuf,
}

impl DbLaunchSource {
    pub fn new(db: Arc<Mutex<Connection>>, default_user_data_root: PathBuf) -> Self {
        Self {
            db,
            default_user_data_root,
        }
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while the lock was held cannot leave SQLite inconsistent (transactions roll
        // back on drop), so a poisoned lock is still usable.
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl LaunchSource for DbLaunchSource {
    fn resolve(&self, profile_id: &str) -> AppResult<LaunchRequest> {
        let conn = self.conn();
        let profile = profiles::get(&conn, profile_id)?;
        if profile.deleted_at.is_some() {
            return Err(AppError::invalid_input(format!(
                "profile {profile_id} is in the trash"
            )));
        }
        let kernel = match &profile.draft.kernel_id {
            Some(kernel_id) => kernels::get(&conn, kernel_id)?,
            None => kernels::get_default(&conn)?
                .ok_or_else(|| AppError::new(ErrorCode::NoKernel, "no kernel is configured"))?,
        };
        let proxy = profile
            .draft
            .proxy_id
            .as_deref()
            .map(|proxy_id| proxies::get(&conn, proxy_id))
            .transpose()?;
        let root = settings::get(&conn)?
            .user_data_root
            .map_or_else(|| self.default_user_data_root.clone(), PathBuf::from);
        drop(conn);

        let user_data_dir = user_data_dir(&profile, &root);
        let plan = build_launch_plan(
            &profile.draft,
            &kernel,
            proxy.as_ref(),
            &user_data_dir,
            HostPlatform::current(),
        )?;
        Ok(LaunchRequest {
            kernel_id: kernel.id,
            plan,
            user_data_dir,
        })
    }

    fn launched(&self, profile_id: &str) -> AppResult<()> {
        profiles::mark_launched(&self.conn(), profile_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profiles::ProfileDraft;
    use crate::db::settings::SettingsPatch;
    use crate::db::test_support::TestDb;

    /// A source over a second connection to `db`, which must outlive it.
    fn source(db: &TestDb, root: &Path) -> (DbLaunchSource, Arc<Mutex<Connection>>) {
        let conn = Arc::new(Mutex::new(db.reopen()));
        (
            DbLaunchSource::new(Arc::clone(&conn), root.to_owned()),
            conn,
        )
    }

    #[test]
    fn resolves_default_kernel_proxy_and_directory() {
        let db = TestDb::new();
        let kernel = db.kernel("chrome", 144);
        let proxy = db.proxy("127.0.0.1", 1080);
        let profile = profiles::create(
            &db.conn,
            ProfileDraft {
                proxy_id: Some(proxy.id.clone()),
                fingerprint_seed: Some(7),
                ..TestDb::draft("p")
            },
        )
        .unwrap();
        let root = PathBuf::from(db.abs_path("default-root"));
        let (source, conn) = source(&db, &root);

        let request = source.resolve(&profile.id).unwrap();
        let udd = root.join(&profile.id);
        assert_eq!(request.kernel_id, kernel.id);
        assert_eq!(request.user_data_dir, udd);
        assert_eq!(request.plan.program, PathBuf::from(&kernel.exec_path));
        assert_eq!(
            request.plan.args,
            [
                format!("--user-data-dir={}", udd.display()),
                "--fingerprint=7".to_owned(),
                "--disable-non-proxied-udp".to_owned(),
                "--proxy-server=socks5://127.0.0.1:1080".to_owned(),
            ]
        );

        // The setting overrides the default root; the profile's own directory overrides both.
        let custom_root = db.abs_path("custom-root");
        settings::update(
            &conn.lock().unwrap(),
            SettingsPatch {
                user_data_root: Some(Some(custom_root.clone())),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        assert_eq!(
            source.resolve(&profile.id).unwrap().user_data_dir,
            Path::new(&custom_root).join(&profile.id)
        );
        let own = db.abs_path("own-dir");
        profiles::update(
            &conn.lock().unwrap(),
            &profile.id,
            ProfileDraft {
                user_data_dir: Some(own.clone()),
                ..profile.draft.clone()
            },
        )
        .unwrap();
        assert_eq!(
            source.resolve(&profile.id).unwrap().user_data_dir,
            PathBuf::from(own)
        );

        source.launched(&profile.id).unwrap();
        let launched = profiles::get(&conn.lock().unwrap(), &profile.id).unwrap();
        assert!(launched.last_launched_at.is_some());
    }

    #[test]
    fn unlaunchable_profiles() {
        let db = TestDb::new();
        let profile = db.profile("p");
        let trashed = db.profile("t");
        profiles::soft_delete(&db.conn, &trashed.id).unwrap();
        let (source, _conn) = source(&db, Path::new("/root"));

        let code = |id: &str| source.resolve(id).unwrap_err().code;
        assert_eq!(code(&profile.id), ErrorCode::NoKernel);
        assert_eq!(code("missing"), ErrorCode::NotFound);
        assert_eq!(code(&trashed.id), ErrorCode::InvalidInput);
        assert_eq!(
            source.launched("missing").unwrap_err().code,
            ErrorCode::NotFound
        );
    }
}

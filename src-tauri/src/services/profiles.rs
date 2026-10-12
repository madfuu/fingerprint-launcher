//! Profile operations that span more than one repository: who may use which user data
//! directory, what may happen to a running profile, copying, and deleting with the data.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;

use crate::db::profiles::{self, Profile, ProfileDraft, ProfileQuery, ProfileRow, SEED_MAX};
use crate::db::settings::{self, Locale};
use crate::db::{Page, in_tx, new_id};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::paths::{CASE_INSENSITIVE_FS, dir_contains, dirs_overlap, strictly_inside};
use crate::services::launcher::{user_data_dir, user_data_root};
use crate::state::AppState;

/// What `purgeProfile` did with the browser data directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PurgeResult {
    /// The profile's user data directory (kept unless `dataDir` is `removed`).
    pub user_data_dir: String,
    pub data_dir: DataDirOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DataDirOutcome {
    Removed,
    /// `dropUserData` was false.
    NotRequested,
    /// There was no directory to delete.
    Missing,
    /// Kept: the directory is not strictly inside the user data root (a custom location, the
    /// root itself, or a symlink). The launcher never deletes those.
    OutsideRoot,
    /// Kept: another profile's directory is the same, inside it, or contains it.
    Shared,
}

/// A page of the profile list; the `running` view uses the launcher's table.
pub async fn list(state: &AppState, query: ProfileQuery) -> AppResult<Page<ProfileRow>> {
    let running = state.launcher.running_ids();
    state
        .db(move |conn| profiles::list(conn, &query, &running))
        .await
}

pub async fn create(state: &AppState, draft: ProfileDraft) -> AppResult<Profile> {
    let default_root = state.paths.default_user_data_root();
    state
        .db(move |conn| create_checked(conn, &default_root, draft))
        .await
}

pub async fn update(state: &AppState, id: String, draft: ProfileDraft) -> AppResult<Profile> {
    let default_root = state.paths.default_user_data_root();
    state
        .db(move |conn| {
            in_tx(conn, |conn| {
                let root = user_data_root(conn, &default_root)?;
                check_dir_is_free(conn, &root, &own_dir(&draft, &root, &id), Some(&id))?;
                profiles::update(conn, &id, draft)
            })
        })
        .await
}

/// Moves a profile to the trash; refused while it runs.
pub async fn soft_delete(state: &AppState, id: String) -> AppResult<()> {
    ensure_not_running(state, &id)?;
    state.db(move |conn| profiles::soft_delete(conn, &id)).await
}

/// Deletes a profile for good; refused while it runs. With `drop_user_data` its browser data
/// directory goes too, but only when it lies strictly inside the user data root and no other
/// profile shares it (design §5): a directory the user put elsewhere is never deleted.
pub async fn purge(state: &AppState, id: String, drop_user_data: bool) -> AppResult<PurgeResult> {
    ensure_not_running(state, &id)?;
    let default_root = state.paths.default_user_data_root();
    let lookup_id = id.clone();
    let (dir, decision) = state
        .db(move |conn| {
            let profile = profiles::get(conn, &lookup_id)?;
            let root = user_data_root(conn, &default_root)?;
            let dir = user_data_dir(&profile, &root);
            if !drop_user_data {
                return Ok((dir, Err(DataDirOutcome::NotRequested)));
            }
            let others: Vec<PathBuf> = profiles::dir_owners(conn)?
                .into_iter()
                .filter(|owner| owner.id != lookup_id)
                .map(|owner| owned_dir(owner.user_data_dir.as_deref(), &root, &owner.id))
                .collect();
            let decision = removable_dir(&dir, &root, &others)?;
            Ok((dir, decision))
        })
        .await?;

    // Outside the database lock: a browser profile can be large.
    let data_dir = match decision {
        Ok(real_dir) => {
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&real_dir))
                .await
                .map_err(|err| AppError::new(ErrorCode::Io, err.to_string()))?
                .map_err(|err| {
                    AppError::new(
                        ErrorCode::Io,
                        format!("cannot delete user data directory: {err}"),
                    )
                })?;
            DataDirOutcome::Removed
        }
        Err(kept) => kept,
    };

    let row_id = id.clone();
    state.db(move |conn| profiles::purge(conn, &row_id)).await?;
    // Best effort: the log only matters while the profile exists.
    let _ = tokio::fs::remove_file(state.paths.log_dir().join(format!("{id}.log"))).await;
    Ok(PurgeResult {
        user_data_dir: dir.to_string_lossy().into_owned(),
        data_dir,
    })
}

/// A copy with a new id, a new seed (when the original had one), the default user data
/// directory and a "copy" name in the UI language (R1.4).
pub async fn duplicate(state: &AppState, id: String) -> AppResult<Profile> {
    let default_root = state.paths.default_user_data_root();
    state
        .db(move |conn| {
            let original = profiles::get(conn, &id)?;
            let locale = settings::get(conn)?.locale;
            let draft = ProfileDraft {
                name: copy_name(&original.draft.name, locale),
                fingerprint_seed: original.draft.fingerprint_seed.map(|_| random_seed()),
                user_data_dir: None,
                ..original.draft
            };
            create_checked(conn, &default_root, draft)
        })
        .await
}

/// Saves a new profile after checking that its user data directory is its own. Composes
/// inside a caller's transaction (import).
pub(crate) fn create_checked(
    conn: &Connection,
    default_root: &Path,
    draft: ProfileDraft,
) -> AppResult<Profile> {
    in_tx(conn, |conn| {
        let root = user_data_root(conn, default_root)?;
        // The default directory of a new profile is `<root>/<its new id>`; any fresh id stands
        // in for it, since only a directory at or above it can overlap.
        check_dir_is_free(conn, &root, &own_dir(&draft, &root, &new_id()), None)?;
        profiles::create(conn, draft)
    })
}

/// A random `--fingerprint` seed in `1..=SEED_MAX`.
pub fn random_seed() -> u32 {
    // The first four bytes of a v4 UUID are fully random (from the OS generator).
    let bytes = uuid::Uuid::new_v4().into_bytes();
    1 + u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % SEED_MAX
}

fn copy_name(name: &str, locale: Locale) -> String {
    match locale {
        Locale::ZhCn => format!("{name} 副本"),
        Locale::En => format!("{name} (copy)"),
    }
}

fn ensure_not_running(state: &AppState, id: &str) -> AppResult<()> {
    if state.launcher.is_running(id) {
        return Err(AppError::new(
            ErrorCode::ProfileRunning,
            format!("profile {id} is running; stop it first"),
        ));
    }
    Ok(())
}

/// The directory a draft would use as profile `id`.
fn own_dir(draft: &ProfileDraft, root: &Path, id: &str) -> PathBuf {
    let custom = draft
        .user_data_dir
        .as_deref()
        .map(str::trim)
        .filter(|dir| !dir.is_empty());
    owned_dir(custom, root, id)
}

fn owned_dir(custom: Option<&str>, root: &Path, id: &str) -> PathBuf {
    custom.map_or_else(|| root.join(id), PathBuf::from)
}

/// `conflict` when `dir` is, contains or lies inside the directory of another profile
/// (trashed ones included), or is the user data root or above it. Chromium allows one browser
/// per directory, and deleting one profile's data would destroy the other's; a directory
/// holding the root would hold every default directory, present and future.
fn check_dir_is_free(
    conn: &Connection,
    root: &Path,
    dir: &Path,
    exclude_id: Option<&str>,
) -> AppResult<()> {
    // Only a custom directory can: a default one is below the root.
    if dir_contains(dir, root, CASE_INSENSITIVE_FS) {
        return Err(AppError::conflict(format!(
            "user data directory {} contains the user data root {}",
            dir.display(),
            root.display()
        )));
    }
    for owner in profiles::dir_owners(conn)? {
        if exclude_id == Some(owner.id.as_str()) {
            continue;
        }
        let theirs = owned_dir(owner.user_data_dir.as_deref(), root, &owner.id);
        if dirs_overlap(dir, &theirs, CASE_INSENSITIVE_FS) {
            return Err(AppError::conflict(format!(
                "user data directory {} overlaps {} of profile \"{}\"",
                dir.display(),
                theirs.display(),
                owner.name
            )));
        }
    }
    Ok(())
}

/// The real path to delete for `dir`, or why it is kept. Compares canonical paths, so `..`
/// and symlinks (in `dir`, the root or another profile's directory, including macOS's
/// `/var` -> `/private/var`) cannot lead outside the root or into another profile's data.
fn removable_dir(
    dir: &Path,
    root: &Path,
    others: &[PathBuf],
) -> AppResult<Result<PathBuf, DataDirOutcome>> {
    let meta = match std::fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Err(DataDirOutcome::Missing));
        }
        Err(err) => return Err(err.into()),
    };
    if meta.file_type().is_symlink() {
        // Its data lives wherever the link points; that is not ours to delete.
        return Ok(Err(DataDirOutcome::OutsideRoot));
    }
    if !meta.is_dir() {
        return Ok(Err(DataDirOutcome::Missing));
    }
    let Ok(real_root) = std::fs::canonicalize(root) else {
        return Ok(Err(DataDirOutcome::OutsideRoot));
    };
    let real_dir = std::fs::canonicalize(dir)?;
    if !strictly_inside(&real_dir, &real_root) {
        return Ok(Err(DataDirOutcome::OutsideRoot));
    }
    for other in others {
        let shared = dirs_overlap(dir, other, CASE_INSENSITIVE_FS)
            || std::fs::canonicalize(other).is_ok_and(|real_other| {
                real_other.starts_with(&real_dir) || real_dir.starts_with(&real_other)
            });
        if shared {
            return Ok(Err(DataDirOutcome::Shared));
        }
    }
    Ok(Ok(real_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::groups::{self, GroupInput};
    use crate::db::settings::SettingsPatch;
    use crate::db::tags::{self, TagInput};
    use crate::state::test_support::TestState;

    fn draft(name: &str) -> ProfileDraft {
        ProfileDraft {
            name: name.into(),
            ..ProfileDraft::default()
        }
    }

    fn with_dir(name: &str, dir: &Path) -> ProfileDraft {
        ProfileDraft {
            user_data_dir: Some(dir.to_str().unwrap().to_owned()),
            ..draft(name)
        }
    }

    fn default_dir(t: &TestState, profile: &Profile) -> PathBuf {
        t.state.paths.default_user_data_root().join(&profile.id)
    }

    #[tokio::test]
    async fn user_data_directories_must_not_overlap() {
        let t = TestState::new();
        let state = &t.state;
        let custom = t.dir.path().join("custom");
        let a = create(state, with_dir("a", &custom)).await.unwrap();
        let b = create(state, draft("b")).await.unwrap();

        let code = |r: AppResult<Profile>| r.unwrap_err().code;
        // The same directory, written differently.
        let same = custom.join(".").join("x").join("..");
        assert_eq!(
            code(create(state, with_dir("c", &same)).await),
            ErrorCode::Conflict
        );
        // Inside or around another profile's directory.
        assert_eq!(
            code(create(state, with_dir("c", &custom.join("sub"))).await),
            ErrorCode::Conflict
        );
        assert_eq!(
            code(create(state, with_dir("c", t.dir.path())).await),
            ErrorCode::Conflict
        );
        // Another profile's default directory.
        assert_eq!(
            code(create(state, with_dir("c", &default_dir(&t, &b))).await),
            ErrorCode::Conflict
        );
        // The root: it contains every default directory.
        let root = state.paths.default_user_data_root();
        assert_eq!(
            code(update(state, a.id.clone(), with_dir("a", &root)).await),
            ErrorCode::Conflict
        );
        if CASE_INSENSITIVE_FS {
            let upper = PathBuf::from(custom.to_str().unwrap().to_uppercase());
            assert_eq!(
                code(create(state, with_dir("c", &upper)).await),
                ErrorCode::Conflict
            );
        }

        // Trashed profiles keep their directory.
        profiles_soft_delete(&t, &a.id).await;
        assert_eq!(
            code(create(state, with_dir("c", &custom)).await),
            ErrorCode::Conflict
        );

        // A profile keeps its own directory on update, and siblings are fine.
        update(state, a.id.clone(), with_dir("renamed", &custom))
            .await
            .unwrap();
        create(state, with_dir("c", &t.dir.path().join("custom2")))
            .await
            .unwrap();
        create(state, draft("d")).await.unwrap();
    }

    async fn profiles_soft_delete(t: &TestState, id: &str) {
        soft_delete(&t.state, id.to_owned()).await.unwrap();
    }

    #[tokio::test]
    async fn a_custom_directory_cannot_hold_the_root() {
        // Even with no default directory in use yet: it would hold every future one.
        let t = TestState::new();
        let state = &t.state;
        let root = state.paths.default_user_data_root();
        for dir in [root.clone(), root.join(".."), t.dir.path().to_owned()] {
            let err = create(state, with_dir("p", &dir)).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::Conflict, "{}", dir.display());
        }
        // Below the root is fine.
        create(state, with_dir("p", &root.join("shop")))
            .await
            .unwrap();
        create(state, draft("default dir")).await.unwrap();
    }

    #[tokio::test]
    async fn duplicate_gets_new_identity_and_keeps_settings() {
        let t = TestState::new();
        let state = &t.state;
        let tag = state
            .db(|conn| {
                tags::create(
                    conn,
                    TagInput {
                        name: "vip".into(),
                        color: "#fff".into(),
                    },
                )
            })
            .await
            .unwrap();
        let group = state
            .db(|conn| {
                groups::create(
                    conn,
                    GroupInput {
                        name: "Work".into(),
                        color: None,
                        sort_order: 0,
                    },
                )
            })
            .await
            .unwrap();
        let original = create(
            state,
            ProfileDraft {
                fingerprint_seed: Some(1000),
                timezone: Some("Asia/Shanghai".into()),
                group_id: Some(group.id.clone()),
                tag_ids: vec![tag.id.clone()],
                ..with_dir("Shop", &t.dir.path().join("shop"))
            },
        )
        .await
        .unwrap();

        let copy = duplicate(state, original.id.clone()).await.unwrap();
        assert_ne!(copy.id, original.id);
        assert_eq!(copy.draft.name, "Shop 副本");
        assert_eq!(copy.draft.user_data_dir, None);
        let seed = copy.draft.fingerprint_seed.unwrap();
        assert!((1..=SEED_MAX).contains(&seed));
        assert_eq!(copy.draft.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(copy.draft.group_id, Some(group.id));
        assert_eq!(copy.draft.tag_ids, [tag.id]);

        state
            .db(|conn| {
                settings::update(
                    conn,
                    SettingsPatch {
                        locale: Some(Locale::En),
                        ..SettingsPatch::default()
                    },
                )
            })
            .await
            .unwrap();
        let plain = create(state, draft("Plain")).await.unwrap();
        let copy = duplicate(state, plain.id).await.unwrap();
        assert_eq!(copy.draft.name, "Plain (copy)");
        // No seed stays no seed: copying must not switch fingerprinting on.
        assert_eq!(copy.draft.fingerprint_seed, None);

        let err = duplicate(state, "missing".into()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn random_seeds_are_in_range() {
        for _ in 0..1000 {
            assert!((1..=SEED_MAX).contains(&random_seed()));
        }
    }

    #[tokio::test]
    async fn purge_removes_the_default_directory_and_log() {
        let t = TestState::new();
        let state = &t.state;
        let p = create(state, draft("p")).await.unwrap();
        let dir = default_dir(&t, &p);
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(dir.join("Default").join("Cookies"), "x").unwrap();
        let log = state.paths.log_dir().join(format!("{}.log", p.id));
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "log").unwrap();

        let result = purge(state, p.id.clone(), true).await.unwrap();
        assert_eq!(result.data_dir, DataDirOutcome::Removed);
        assert_eq!(PathBuf::from(&result.user_data_dir), dir);
        assert!(!dir.exists());
        assert!(!log.exists());
        let err = state
            .db(move |conn| profiles::get(conn, &p.id))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn purge_keeps_directories_it_does_not_own() {
        let t = TestState::new();
        let state = &t.state;
        let root = state.paths.default_user_data_root();

        // Not requested.
        let p = create(state, draft("p")).await.unwrap();
        std::fs::create_dir_all(default_dir(&t, &p)).unwrap();
        let result = purge(state, p.id.clone(), false).await.unwrap();
        assert_eq!(result.data_dir, DataDirOutcome::NotRequested);
        assert!(default_dir(&t, &p).exists());

        // Never created.
        let p = create(state, draft("p")).await.unwrap();
        let result = purge(state, p.id, true).await.unwrap();
        assert_eq!(result.data_dir, DataDirOutcome::Missing);

        // A custom directory outside the root.
        let outside = t.dir.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        let p = create(state, with_dir("p", &outside)).await.unwrap();
        let result = purge(state, p.id, true).await.unwrap();
        assert_eq!(result.data_dir, DataDirOutcome::OutsideRoot);
        assert_eq!(PathBuf::from(&result.user_data_dir), outside);
        assert!(outside.exists());

        // Escaping the root with `..` (root is `<tmp>/data/profiles`).
        let sneaky = root.join("..").join("..").join("elsewhere");
        let p = create(state, with_dir("p", &sneaky)).await.unwrap();
        assert_eq!(
            purge(state, p.id, true).await.unwrap().data_dir,
            DataDirOutcome::OutsideRoot
        );
        assert!(outside.exists());
    }

    /// Sets a profile's directory behind the save-time check, as data from before it would.
    async fn plant_dir(state: &AppState, id: &str, dir: &Path) {
        let (id, dir) = (id.to_owned(), dir.to_str().unwrap().to_owned());
        state
            .db(move |conn| {
                conn.execute(
                    "UPDATE profiles SET user_data_dir = ?2 WHERE id = ?1",
                    (&id, &dir),
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn purge_never_deletes_the_root_or_shared_data() {
        let t = TestState::new();
        let state = &t.state;
        let root = state.paths.default_user_data_root();
        std::fs::create_dir_all(&root).unwrap();

        let a = create(state, draft("a")).await.unwrap();
        plant_dir(state, &a.id, &root).await;
        assert_eq!(
            purge(state, a.id, true).await.unwrap().data_dir,
            DataDirOutcome::OutsideRoot
        );
        assert!(root.exists());

        let b = create(state, draft("b")).await.unwrap();
        let c = create(state, draft("c")).await.unwrap();
        let b_dir = default_dir(&t, &b);
        std::fs::create_dir_all(&b_dir).unwrap();
        plant_dir(state, &c.id, &b_dir).await;
        assert_eq!(
            purge(state, b.id, true).await.unwrap().data_dir,
            DataDirOutcome::Shared
        );
        assert!(b_dir.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn purge_does_not_follow_symlinks_out_of_the_root() {
        use std::os::unix::fs::symlink;

        let t = TestState::new();
        let state = &t.state;
        let root = state.paths.default_user_data_root();
        let target = t.dir.path().join("precious");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("file"), "keep").unwrap();

        // The profile directory itself is a link out of the root.
        let p = create(state, draft("p")).await.unwrap();
        std::fs::create_dir_all(&root).unwrap();
        symlink(&target, default_dir(&t, &p)).unwrap();
        assert_eq!(
            purge(state, p.id, true).await.unwrap().data_dir,
            DataDirOutcome::OutsideRoot
        );

        // A link higher up the path.
        std::fs::create_dir_all(target.join("inner")).unwrap();
        symlink(&target, root.join("link")).unwrap();
        let p = create(state, with_dir("p", &root.join("link").join("inner")))
            .await
            .unwrap();
        assert_eq!(
            purge(state, p.id, true).await.unwrap().data_dir,
            DataDirOutcome::OutsideRoot
        );
        assert!(target.join("file").exists());
        assert!(target.join("inner").exists());
    }

    #[tokio::test]
    async fn purge_follows_a_symlinked_root() {
        // Like macOS's /var -> /private/var: the root is reached through a link, and its real
        // path is what the directory is compared with.
        let t = TestState::new();
        let state = &t.state;
        let real_root = t.dir.path().join("real-root");
        std::fs::create_dir_all(&real_root).unwrap();
        let linked_root = t.dir.path().join("linked-root");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_root, &linked_root).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&real_root, &linked_root).is_err() {
            return; // creating symlinks needs developer mode or admin rights
        }
        let root_text = linked_root.to_str().unwrap().to_owned();
        state
            .db(move |conn| {
                settings::update(
                    conn,
                    SettingsPatch {
                        user_data_root: Some(Some(root_text)),
                        ..SettingsPatch::default()
                    },
                )
            })
            .await
            .unwrap();
        let p = create(state, draft("p")).await.unwrap();
        std::fs::create_dir_all(real_root.join(&p.id)).unwrap();
        assert_eq!(
            purge(state, p.id.clone(), true).await.unwrap().data_dir,
            DataDirOutcome::Removed
        );
        assert!(!real_root.join(&p.id).exists());
        assert!(real_root.exists());
    }
}

//! Where the launcher keeps its files (design §2.1), and how user data directories are compared.

use std::path::{Component, Path, PathBuf};

use crate::db::DB_FILE_NAME;

/// The launcher's files under the platform's app data directory:
///
/// ```text
/// <data_dir>/launcher.db
/// <data_dir>/profiles/<profile id>/   default user data directories
/// <data_dir>/logs/profiles/<id>.log   browser stderr
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    data_dir: PathBuf,
}

impl AppPaths {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join(DB_FILE_NAME)
    }

    /// Root of the default user data directories while the `user_data_root` setting is unset.
    pub fn default_user_data_root(&self) -> PathBuf {
        self.data_dir.join("profiles")
    }

    /// Per-profile browser stderr logs.
    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs").join("profiles")
    }
}

/// Whether directory names differ only in case on this platform's default file systems
/// (NTFS, APFS). Linux file systems are case-sensitive.
pub(crate) const CASE_INSENSITIVE_FS: bool = cfg!(any(windows, target_os = "macos"));

/// Whether one directory is the other or contains it, compared lexically: `.` and `..` are
/// resolved without touching the file system, trailing separators ignored, and with
/// `fold_case` letter case too. Symlinks are not resolved; callers that delete files compare
/// canonical paths as well.
pub(crate) fn dirs_overlap(a: &Path, b: &Path, fold_case: bool) -> bool {
    let a = dir_key(a, fold_case);
    let b = dir_key(b, fold_case);
    a.starts_with(&b) || b.starts_with(&a)
}

/// Whether `outer` is `inner` or one of its ancestors, compared lexically like
/// [`dirs_overlap`].
pub(crate) fn dir_contains(outer: &Path, inner: &Path, fold_case: bool) -> bool {
    dir_key(inner, fold_case).starts_with(&dir_key(outer, fold_case))
}

/// Whether two paths name the same file, compared lexically like [`dirs_overlap`].
pub(crate) fn same_path(a: &Path, b: &Path, fold_case: bool) -> bool {
    dir_key(a, fold_case) == dir_key(b, fold_case)
}

/// The normalized components of `path`.
fn dir_key(path: &Path, fold_case: bool) -> Vec<String> {
    let text = |s: &std::ffi::OsStr| {
        let s = s.to_string_lossy();
        if fold_case {
            s.to_lowercase()
        } else {
            s.into_owned()
        }
    };
    let mut key: Vec<String> = Vec::new();
    // Leading entries `..` cannot remove: the prefix and root, or `..`s of a relative path.
    let mut fixed = 0;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                key.push(text(prefix.as_os_str()));
                fixed += 1;
            }
            Component::RootDir => {
                key.push(std::path::MAIN_SEPARATOR_STR.to_owned());
                fixed += 1;
            }
            Component::CurDir => {}
            Component::ParentDir if key.len() > fixed => {
                key.pop();
            }
            Component::ParentDir => {
                // Above the root `..` is the root itself; a relative path keeps it.
                if !path.has_root() {
                    key.push("..".to_owned());
                    fixed += 1;
                }
            }
            Component::Normal(name) => key.push(text(name)),
        }
    }
    key
}

/// Whether `child` lies strictly below `root` (never `root` itself). Both must already be
/// canonical, so symlinks cannot fake the relationship.
pub(crate) fn strictly_inside(child: &Path, root: &Path) -> bool {
    child != root && child.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let paths = AppPaths::new(PathBuf::from("/data"));
        assert_eq!(paths.db_file(), Path::new("/data").join("launcher.db"));
        assert_eq!(
            paths.default_user_data_root(),
            Path::new("/data").join("profiles")
        );
        assert_eq!(
            paths.log_dir(),
            Path::new("/data").join("logs").join("profiles")
        );
    }

    #[test]
    fn overlap_is_equality_or_containment_after_normalization() {
        let overlap = |a: &str, b: &str| dirs_overlap(Path::new(a), Path::new(b), false);
        assert!(overlap("/data/p1", "/data/p1"));
        assert!(overlap("/data/p1/", "/data/./p1"));
        assert!(overlap("/data/x/../p1", "/data/p1"));
        assert!(overlap("/data", "/data/p1"));
        assert!(overlap("/data/p1/sub", "/data/p1"));
        assert!(!overlap("/data/p1", "/data/p10"));
        assert!(!overlap("/data/p1", "/data/p2"));
        assert!(!overlap("/data/P1", "/data/p1"));
        assert!(dirs_overlap(
            Path::new("/data/P1"),
            Path::new("/data/p1"),
            true
        ));
        // `..` stops at the root.
        assert!(overlap("/../data", "/data"));
    }

    #[test]
    fn containment_is_one_way() {
        let contains = |a: &str, b: &str| dir_contains(Path::new(a), Path::new(b), false);
        assert!(contains("/data", "/data/p1"));
        assert!(contains("/data/", "/data/./x/.."));
        assert!(!contains("/data/p1", "/data"));
        assert!(!contains("/data/p", "/data/p1"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_fold_case_and_separators() {
        assert!(dirs_overlap(
            Path::new(r"C:\Users\A\Profiles\p1"),
            Path::new("c:/users/a/profiles/P1/"),
            true
        ));
        assert!(!dirs_overlap(
            Path::new(r"C:\data\p1"),
            Path::new(r"D:\data\p1"),
            true
        ));
    }

    #[test]
    fn strictly_inside_excludes_the_root() {
        assert!(strictly_inside(Path::new("/r/a"), Path::new("/r")));
        assert!(strictly_inside(Path::new("/r/a/b"), Path::new("/r")));
        assert!(!strictly_inside(Path::new("/r"), Path::new("/r")));
        assert!(!strictly_inside(Path::new("/ra"), Path::new("/r")));
        assert!(!strictly_inside(Path::new("/x/a"), Path::new("/r")));
    }
}

//! Registering kernels (R7.1, design §5): adding one from a path with its version probed,
//! and listing installed candidates for the user to confirm.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Semaphore;

use crate::db::kernels::{self, Kernel, KernelInput};
use crate::db::require_absolute;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::kernel::probe::{
    KernelProbe, ProbeMethod, VERSION_FLAG_TIMEOUT, bundle_executable, probe_kernel,
};
use crate::paths::{CASE_INSENSITIVE_FS, same_path};
use crate::services::launcher::check_executable;
use crate::state::AppState;

/// `--version` limit per candidate while detecting: a candidate that hangs must not hold up
/// the list.
const DETECT_VERSION_TIMEOUT: Duration = Duration::from_secs(3);
/// Candidates probed at the same time.
const DETECT_PARALLELISM: usize = 4;

/// An installed browser that may be a fingerprint-chromium kernel (`detectKernels`). Only
/// listed: the user decides, since a stock Chromium looks the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KernelCandidate {
    /// The executable, as `addKernel` takes it.
    pub path: String,
    pub major_version: Option<u32>,
    pub full_version: Option<String>,
    pub method: Option<ProbeMethod>,
    /// A kernel with this path is already registered.
    pub registered: bool,
}

/// Registers the kernel at `path`: a `.app` bundle is resolved to its executable, which must
/// exist and be executable. The version is probed; `major_version` overrides it, and is
/// required (`kernel_version_unknown`) when probing finds nothing. `name` defaults to
/// "Chromium <version>".
pub async fn add_kernel(
    state: &AppState,
    path: String,
    name: Option<String>,
    major_version: Option<u32>,
) -> AppResult<Kernel> {
    let exec = resolve_executable(&path).await?;
    let probe = probe_kernel(&exec, VERSION_FLAG_TIMEOUT).await;
    let input = kernel_input(&exec, name, major_version, probe)?;
    state.db(move |conn| kernels::create(conn, input)).await
}

/// The version of the kernel at `path` (all fields `None` when it cannot be found). The path
/// must be absolute: a bare name would be looked up on `PATH` and run with `--version`.
pub async fn probe(path: &str) -> AppResult<KernelProbe> {
    let path = path.trim();
    require_absolute(path, "kernel path")?;
    Ok(probe_kernel(Path::new(path), VERSION_FLAG_TIMEOUT).await)
}

/// Replaces a kernel's fields. A changed path is checked like [`add_kernel`] checks it; an
/// unchanged one is not, so a kernel whose file went away can still be renamed.
pub async fn update_kernel(
    state: &AppState,
    id: String,
    mut input: KernelInput,
) -> AppResult<Kernel> {
    let lookup = id.clone();
    let current = state.db(move |conn| kernels::get(conn, &lookup)).await?;
    if input.exec_path.trim() != current.exec_path {
        input.exec_path = path_text(&resolve_executable(&input.exec_path).await?)?;
    }
    state
        .db(move |conn| kernels::update(conn, &id, input))
        .await
}

/// Installed candidates (research/fingerprint-chromium-cli.md §8), probed in parallel.
pub async fn detect_kernels(state: &AppState) -> AppResult<Vec<KernelCandidate>> {
    match CandidateOs::current() {
        Some(os) => detect_in(state, os, CandidateRoots::from_env()).await,
        None => Ok(Vec::new()),
    }
}

async fn detect_in(
    state: &AppState,
    os: CandidateOs,
    roots: CandidateRoots,
) -> AppResult<Vec<KernelCandidate>> {
    let registered: Vec<PathBuf> = state
        .db(kernels::list)
        .await?
        .into_iter()
        .map(|kernel| PathBuf::from(kernel.exec_path))
        .collect();
    let paths = tokio::task::spawn_blocking(move || candidate_paths(os, &roots))
        .await
        .map_err(|err| AppError::new(ErrorCode::Io, err.to_string()))?;

    let permits = Arc::new(Semaphore::new(DETECT_PARALLELISM));
    let mut probes = tokio::task::JoinSet::new();
    for (index, path) in paths.into_iter().enumerate() {
        let permits = Arc::clone(&permits);
        probes.spawn(async move {
            let _permit = permits.acquire_owned().await;
            let probe = probe_kernel(&path, DETECT_VERSION_TIMEOUT).await;
            (index, path, probe)
        });
    }
    let mut found = Vec::new();
    while let Some(joined) = probes.join_next().await {
        let Ok((index, path, probe)) = joined else {
            continue;
        };
        let registered = registered
            .iter()
            .any(|known| same_path(known, &path, CASE_INSENSITIVE_FS));
        found.push((
            index,
            KernelCandidate {
                path: path.to_string_lossy().into_owned(),
                major_version: probe.major_version,
                full_version: probe.full_version,
                method: probe.method,
                registered,
            },
        ));
    }
    found.sort_by_key(|(index, _)| *index);
    Ok(found.into_iter().map(|(_, candidate)| candidate).collect())
}

/// The executable to register for a user-picked `path`.
async fn resolve_executable(path: &str) -> AppResult<PathBuf> {
    let path = path.trim();
    require_absolute(path, "kernel path")?;
    let path = Path::new(path);
    let exec = match bundle_executable(path).await {
        Some(exec) => exec,
        None if is_app_bundle(path) => {
            return Err(AppError::new(
                ErrorCode::KernelNotExecutable,
                format!("{} is not an app bundle with an executable", path.display()),
            ));
        }
        None => path.to_owned(),
    };
    check_executable(&exec)?;
    Ok(exec)
}

fn is_app_bundle(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
}

fn kernel_input(
    exec: &Path,
    name: Option<String>,
    major_version: Option<u32>,
    probe: KernelProbe,
) -> AppResult<KernelInput> {
    let major = major_version.or(probe.major_version).ok_or_else(|| {
        AppError::new(
            ErrorCode::KernelVersionUnknown,
            format!(
                "cannot detect the version of {}; enter its major version",
                exec.display()
            ),
        )
    })?;
    // The probed full version only describes the kernel if the major version agrees.
    let full_version = probe
        .full_version
        .filter(|_| probe.major_version == Some(major));
    let name = name
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| match &full_version {
            Some(full) => format!("Chromium {full}"),
            None => format!("Chromium {major}"),
        });
    Ok(KernelInput {
        name,
        exec_path: path_text(exec)?,
        major_version: major,
        full_version,
    })
}

fn path_text(path: &Path) -> AppResult<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        AppError::invalid_input(format!(
            "kernel path is not valid UTF-8: {}",
            path.display()
        ))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateOs {
    Linux,
    Windows,
    Macos,
}

impl CandidateOs {
    fn current() -> Option<Self> {
        if cfg!(windows) {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// Where the candidate locations start; injectable for tests.
#[derive(Debug, Clone, Default)]
struct CandidateRoots {
    home: Option<PathBuf>,
    /// `%LOCALAPPDATA%`.
    local_app_data: Option<PathBuf>,
    /// `%ProgramFiles%`.
    program_files: Option<PathBuf>,
    /// `/opt`.
    opt: PathBuf,
    /// `/Applications`.
    applications: PathBuf,
}

impl CandidateRoots {
    fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            home: var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }),
            local_app_data: var("LOCALAPPDATA"),
            program_files: var("ProgramFiles"),
            opt: PathBuf::from("/opt"),
            applications: PathBuf::from("/Applications"),
        }
    }
}

/// Existing files at the candidate locations of research §8, in a stable order:
///
/// - Linux: `~/Applications/*.AppImage` and `~/Downloads/*.AppImage` whose name mentions
///   Chromium, `~/Downloads/ungoogled-chromium-*_linux/chrome`, `/opt/ungoogled-chromium*/chrome`
/// - Windows: `Chromium\Application\chrome.exe` under `%LOCALAPPDATA%` and `%ProgramFiles%`
/// - macOS: `Chromium.app` in `/Applications` and `~/Applications`
///
/// AppImages must mention Chromium in their name: an unrelated AppImage would be started with
/// `--version` when probed.
fn candidate_paths(os: CandidateOs, roots: &CandidateRoots) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    match os {
        CandidateOs::Linux => {
            let is_chromium_appimage = |name: &str| {
                has_suffix_ignore_case(name, ".AppImage")
                    && name.to_ascii_lowercase().contains("chromium")
            };
            if let Some(home) = &roots.home {
                for dir in ["Applications", "Downloads"] {
                    paths.extend(matching_entries(&home.join(dir), is_chromium_appimage));
                }
                paths.extend(subdir_files(
                    &home.join("Downloads"),
                    |name| name.starts_with("ungoogled-chromium-") && name.ends_with("_linux"),
                    "chrome",
                ));
            }
            paths.extend(subdir_files(
                &roots.opt,
                |name| name.starts_with("ungoogled-chromium"),
                "chrome",
            ));
        }
        CandidateOs::Windows => {
            for base in [&roots.local_app_data, &roots.program_files]
                .into_iter()
                .flatten()
            {
                paths.push(base.join("Chromium").join("Application").join("chrome.exe"));
            }
        }
        CandidateOs::Macos => {
            let mut bases = vec![roots.applications.clone()];
            bases.extend(roots.home.as_ref().map(|home| home.join("Applications")));
            for base in bases {
                paths.push(
                    base.join("Chromium.app")
                        .join("Contents")
                        .join("MacOS")
                        .join("Chromium"),
                );
            }
        }
    }
    paths.retain(|path| path.is_file());
    paths.dedup();
    paths
}

/// Entries of `dir` whose name satisfies `keep`, sorted by name.
fn matching_entries(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(&keep))
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
}

/// `<dir>/<subdir>/<file>` for each subdirectory whose name satisfies `keep`.
fn subdir_files(dir: &Path, keep: impl Fn(&str) -> bool, file: &str) -> Vec<PathBuf> {
    matching_entries(dir, keep)
        .into_iter()
        .map(|subdir| subdir.join(file))
        .collect()
}

fn has_suffix_ignore_case(name: &str, suffix: &str) -> bool {
    name.len() >= suffix.len()
        && name.is_char_boundary(name.len() - suffix.len())
        && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support::TestState;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn linux_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let opt = dir.path().join("opt");
        let roots = CandidateRoots {
            home: Some(home.clone()),
            opt: opt.clone(),
            ..CandidateRoots::default()
        };
        let expected = [
            home.join("Applications/ungoogled-chromium-150.0.7871.186-1-x86_64.AppImage"),
            home.join("Downloads/Chromium.appimage"),
            home.join("Downloads/ungoogled-chromium-150.0.7871.186-1-x86_64_linux/chrome"),
            opt.join("ungoogled-chromium/chrome"),
        ];
        for path in &expected {
            touch(path);
        }
        // Not candidates: other AppImages, missing executables, other directories.
        touch(&home.join("Applications/Krita.AppImage"));
        std::fs::create_dir_all(home.join("Downloads/ungoogled-chromium-149_linux")).unwrap();
        touch(&home.join("Downloads/chromium_linux/chrome"));
        touch(&opt.join("google/chrome/chrome"));

        assert_eq!(candidate_paths(CandidateOs::Linux, &roots), expected);
        // Nothing to find without a home or /opt.
        assert!(candidate_paths(CandidateOs::Linux, &CandidateRoots::default()).is_empty());
    }

    #[test]
    fn windows_and_macos_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("Local");
        let program_files = dir.path().join("Program Files");
        let windows = local.join("Chromium/Application/chrome.exe");
        touch(&windows);
        let roots = CandidateRoots {
            local_app_data: Some(local),
            program_files: Some(program_files),
            ..CandidateRoots::default()
        };
        assert_eq!(candidate_paths(CandidateOs::Windows, &roots), [windows]);

        let home = dir.path().join("home");
        let applications = dir.path().join("Applications");
        let user_app = home.join("Applications/Chromium.app/Contents/MacOS/Chromium");
        touch(&user_app);
        let roots = CandidateRoots {
            home: Some(home),
            applications,
            ..CandidateRoots::default()
        };
        assert_eq!(candidate_paths(CandidateOs::Macos, &roots), [user_app]);
    }

    #[test]
    fn version_and_name_defaults() {
        let exec = std::env::temp_dir().join("chrome");
        let probed = KernelProbe {
            major_version: Some(150),
            full_version: Some("150.0.7871.186".into()),
            method: Some(ProbeMethod::VersionFlag),
        };
        let unknown = KernelProbe {
            major_version: None,
            full_version: None,
            method: None,
        };

        let input = kernel_input(&exec, None, None, probed.clone()).unwrap();
        assert_eq!(input.name, "Chromium 150.0.7871.186");
        assert_eq!(input.major_version, 150);
        assert_eq!(input.full_version.as_deref(), Some("150.0.7871.186"));

        // A manual major version that disagrees drops the probed full version.
        let input = kernel_input(&exec, Some(" My kernel ".into()), Some(144), probed).unwrap();
        assert_eq!(input.name, "My kernel");
        assert_eq!((input.major_version, input.full_version), (144, None));

        let input = kernel_input(&exec, Some(" ".into()), Some(139), unknown.clone()).unwrap();
        assert_eq!(input.name, "Chromium 139");

        let err = kernel_input(&exec, None, None, unknown).unwrap_err();
        assert_eq!(err.code, ErrorCode::KernelVersionUnknown);
    }

    #[tokio::test]
    async fn add_kernel_checks_the_executable() {
        let t = TestState::new();
        let state = &t.state;
        let code = |r: AppResult<Kernel>| r.unwrap_err().code;
        let missing = t.dir.path().join("missing").to_str().unwrap().to_owned();
        assert_eq!(
            code(add_kernel(state, missing, None, Some(150)).await),
            ErrorCode::KernelMissing
        );
        assert_eq!(
            code(add_kernel(state, "chrome".into(), None, Some(150)).await),
            ErrorCode::InvalidInput
        );
        let dir = t.dir.path().to_str().unwrap().to_owned();
        assert_eq!(
            code(add_kernel(state, dir, None, Some(150)).await),
            ErrorCode::KernelNotExecutable
        );
        let bundle = t.dir.path().join("Empty.app");
        std::fs::create_dir_all(&bundle).unwrap();
        assert_eq!(
            code(add_kernel(state, bundle.to_str().unwrap().to_owned(), None, None).await),
            ErrorCode::KernelNotExecutable
        );
    }

    #[tokio::test]
    async fn probe_takes_absolute_paths_only() {
        // A bare name would be looked up on PATH and executed.
        for relative in ["chrome", "./chrome", " "] {
            let err = probe(relative).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{relative:?}");
        }
        let missing = std::env::temp_dir().join("fpl-no-such-kernel");
        let found = probe(missing.to_str().unwrap()).await.unwrap();
        assert_eq!(found.major_version, None);
    }

    /// A macOS bundle on any host: the `.app` resolves to its executable and the version comes
    /// from `Info.plist`.
    #[tokio::test]
    async fn add_kernel_resolves_app_bundles() {
        let t = TestState::new();
        let bundle = t.dir.path().join("Chromium.app");
        let contents = bundle.join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Chromium</string>
<key>CFBundleShortVersionString</key><string>148.0.7778.215</string>
</dict></plist>"#,
        )
        .unwrap();
        let exec = contents.join("MacOS").join("Chromium");
        std::fs::write(&exec, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let kernel = add_kernel(&t.state, bundle.to_str().unwrap().to_owned(), None, None)
            .await
            .unwrap();
        assert_eq!(PathBuf::from(&kernel.exec_path), exec);
        assert_eq!(kernel.major_version, 148);
        assert_eq!(kernel.full_version.as_deref(), Some("148.0.7778.215"));
        assert_eq!(kernel.name, "Chromium 148.0.7778.215");
        assert!(kernel.is_default);

        // Renaming does not re-check the path; changing it does.
        let renamed = update_kernel(
            &t.state,
            kernel.id.clone(),
            KernelInput {
                name: "Mac".into(),
                exec_path: kernel.exec_path.clone(),
                major_version: 148,
                full_version: kernel.full_version.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(renamed.name, "Mac");
        std::fs::remove_file(&exec).unwrap();
        let err = update_kernel(
            &t.state,
            kernel.id.clone(),
            KernelInput {
                name: "Mac".into(),
                exec_path: bundle.to_str().unwrap().to_owned(),
                major_version: 148,
                full_version: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::KernelNotExecutable);
    }

    #[tokio::test]
    async fn detection_marks_registered_kernels() {
        let t = TestState::new();
        let opt = t.dir.path().join("opt");
        let first = opt.join("ungoogled-chromium-a").join("chrome");
        let second = opt.join("ungoogled-chromium-b").join("chrome");
        touch(&first);
        touch(&second);
        let first_text = first.to_str().unwrap().to_owned();
        t.state
            .db(move |conn| {
                kernels::create(
                    conn,
                    KernelInput {
                        name: "a".into(),
                        exec_path: first_text,
                        major_version: 150,
                        full_version: None,
                    },
                )
            })
            .await
            .unwrap();
        let roots = CandidateRoots {
            opt,
            ..CandidateRoots::default()
        };

        let candidates = detect_in(&t.state, CandidateOs::Linux, roots)
            .await
            .unwrap();
        // Empty files: nothing to probe, so no version.
        let unknown = |path: &Path, registered| KernelCandidate {
            path: path.to_str().unwrap().to_owned(),
            major_version: None,
            full_version: None,
            method: None,
            registered,
        };
        assert_eq!(candidates, [unknown(&first, true), unknown(&second, false)]);
    }
}

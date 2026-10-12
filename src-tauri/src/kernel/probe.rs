//! Kernel version detection (design §5 `probeKernel`, research/fingerprint-chromium-cli.md §7).
//!
//! The method depends on the kind of file, not on the host OS, so every method is plain Rust
//! that runs (and is tested) everywhere:
//!
//! 1. PE image (`chrome.exe`): the version resource. Never executed: `--version` prints nothing
//!    on Windows.
//! 2. macOS bundle (`X.app`, or `X.app/Contents/MacOS/X`): `CFBundleShortVersionString` of
//!    `Contents/Info.plist`.
//! 3. `*.AppImage`: the version in the release file name
//!    (`ungoogled-chromium-150.0.7871.186-1-x86_64.AppImage`); running it needs FUSE.
//! 4. Otherwise, and when 2 or 3 found nothing: `<path> --version` (`Chromium 150.0.7871.186 ...`).
//!
//! When everything fails the result is empty and the user enters the major version; the
//! launcher never guesses.

mod pe;

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Upper bound for `<kernel> --version`.
pub const VERSION_FLAG_TIMEOUT: Duration = Duration::from_secs(10);

/// How a version was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ProbeMethod {
    PeVersion,
    InfoPlist,
    FileName,
    VersionFlag,
}

/// Result of [`probe_kernel`]. All fields are `None` when the version could not be found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KernelProbe {
    /// Selects the argument set (design §3.2).
    pub major_version: Option<u32>,
    /// Display only, e.g. `150.0.7871.186`.
    pub full_version: Option<String>,
    pub method: Option<ProbeMethod>,
}

impl KernelProbe {
    fn unknown() -> Self {
        Self {
            major_version: None,
            full_version: None,
            method: None,
        }
    }

    fn found(version: Option<String>, method: ProbeMethod) -> Option<Self> {
        let (major, full) = parse_version(&version?)?;
        Some(Self {
            major_version: Some(major),
            full_version: Some(full),
            method: Some(method),
        })
    }
}

/// Finds the Chromium version of the kernel at `path`. `version_timeout` bounds the
/// `--version` run; pass [`VERSION_FLAG_TIMEOUT`].
pub async fn probe_kernel(path: &Path, version_timeout: Duration) -> KernelProbe {
    if starts_like_pe(path).await {
        let version = tokio::fs::read(path)
            .await
            .ok()
            .and_then(|data| pe::file_version(&data));
        return KernelProbe::found(version, ProbeMethod::PeVersion)
            .unwrap_or_else(KernelProbe::unknown);
    }
    if let Some(plist) = info_plist(path) {
        let version = read_plist_string(&plist, "CFBundleShortVersionString").await;
        if let Some(probe) = KernelProbe::found(version, ProbeMethod::InfoPlist) {
            return probe;
        }
    }
    if is_appimage(path) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        if let Some(probe) = KernelProbe::found(name, ProbeMethod::FileName) {
            return probe;
        }
    }
    let version = version_flag(path, version_timeout).await;
    KernelProbe::found(version, ProbeMethod::VersionFlag).unwrap_or_else(KernelProbe::unknown)
}

/// The executable inside a macOS bundle (`X.app/Contents/MacOS/<CFBundleExecutable>`), which is
/// what gets launched when the user picks the `.app` itself. `None` when `app` is not a bundle
/// or names no existing executable.
pub async fn bundle_executable(app: &Path) -> Option<PathBuf> {
    if !has_extension(app, "app") {
        return None;
    }
    let contents = app.join("Contents");
    let name = read_plist_string(&contents.join("Info.plist"), "CFBundleExecutable").await?;
    // A bare file name: anything else would point outside the bundle.
    if Path::new(&name).file_name() != Some(std::ffi::OsStr::new(&name)) {
        return None;
    }
    let executable = contents.join("MacOS").join(name);
    tokio::fs::metadata(&executable)
        .await
        .is_ok_and(|meta| meta.is_file())
        .then_some(executable)
}

/// The first `a.b.c.d` run of digits and dots in `text` (Chromium versions have four parts),
/// as (major, full). A zero major is not a Chromium version.
pub fn parse_version(text: &str) -> Option<(u32, String)> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map(|run| run.trim_matches('.'))
        .find_map(|run| {
            let parts = run
                .split('.')
                .map(|part| part.parse::<u32>().ok())
                .collect::<Option<Vec<_>>>()?;
            match parts[..] {
                [major, _, _, _] if major > 0 => Some((major, run.to_owned())),
                _ => None,
            }
        })
}

async fn starts_like_pe(path: &Path) -> bool {
    let mut head = [0; 2];
    match tokio::fs::File::open(path).await {
        Ok(mut file) => file.read_exact(&mut head).await.is_ok() && pe::is_pe(&head),
        Err(_) => false,
    }
}

/// `Contents/Info.plist` of the bundle `path` is, or is the main executable of.
fn info_plist(path: &Path) -> Option<PathBuf> {
    if has_extension(path, "app") {
        return Some(path.join("Contents").join("Info.plist"));
    }
    let macos = path.parent()?;
    let contents = macos.parent()?;
    let is = |dir: &Path, name: &str| dir.file_name().is_some_and(|n| n == name);
    (is(macos, "MacOS") && is(contents, "Contents")).then(|| contents.join("Info.plist"))
}

async fn read_plist_string(plist: &Path, key: &str) -> Option<String> {
    let data = tokio::fs::read(plist).await.ok()?;
    let value = plist::Value::from_reader(Cursor::new(data)).ok()?;
    Some(value.as_dictionary()?.get(key)?.as_string()?.to_owned())
}

fn is_appimage(path: &Path) -> bool {
    has_extension(path, "AppImage")
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

/// Stdout of `<path> --version`, or `None` if it cannot run, fails or times out (the process
/// is killed then).
async fn version_flag(path: &Path, timeout: Duration) -> Option<String> {
    let child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const X64: &[u8] = include_bytes!("../../tests/fixtures/pe/versioninfo-x64.bin");

    fn probe(path: &Path) -> KernelProbe {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(probe_kernel(path, Duration::from_secs(5)))
    }

    fn found(major: u32, full: &str, method: ProbeMethod) -> KernelProbe {
        KernelProbe {
            major_version: Some(major),
            full_version: Some(full.into()),
            method: Some(method),
        }
    }

    #[test]
    fn parses_chromium_versions() {
        let cases = [
            ("Chromium 150.0.7871.186 built on Debian 12", Some(150)),
            ("Google Chrome for Testing 151.0.7922.34 ", Some(151)),
            (
                "ungoogled-chromium-150.0.7871.186-1-x86_64.AppImage",
                Some(150),
            ),
            ("ungoogled-chromium_148.0.7778.215-1.1_macos.dmg", Some(148)),
            ("v150.0.7871.186.", Some(150)),
            ("150.0.7871", None),
            ("1.2.3.4.5", None),
            ("0.0.0.0", None),
            ("x86_64", None),
            ("", None),
            ("99999999999.0.0.0", None),
        ];
        for (text, major) in cases {
            assert_eq!(parse_version(text).map(|v| v.0), major, "{text}");
        }
        assert_eq!(
            parse_version("Chromium 150.0.7871.186 built").unwrap().1,
            "150.0.7871.186"
        );
    }

    #[test]
    fn pe_file_uses_the_version_resource() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chrome.exe");
        std::fs::write(&path, X64).unwrap();
        assert_eq!(
            probe(&path),
            found(150, "150.0.7871.186", ProbeMethod::PeVersion)
        );
        // A PE without version information is not run as a fallback.
        std::fs::write(&path, b"MZ broken").unwrap();
        assert_eq!(probe(&path), KernelProbe::unknown());
    }

    #[test]
    fn appimage_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("ungoogled-chromium-150.0.7871.186-1-x86_64.AppImage");
        std::fs::write(&path, b"not runnable").unwrap();
        assert_eq!(
            probe(&path),
            found(150, "150.0.7871.186", ProbeMethod::FileName)
        );
        // Renamed: nothing in the name, and the file cannot run.
        let renamed = dir.path().join("chromium.appimage");
        std::fs::rename(&path, &renamed).unwrap();
        assert_eq!(probe(&renamed), KernelProbe::unknown());
    }

    const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>Chromium</string>
  <key>CFBundleShortVersionString</key>
  <string>148.0.7778.215</string>
  <key>CFBundleVersion</key>
  <string>7778.215</string>
</dict>
</plist>
"#;

    #[test]
    fn macos_bundle_info_plist() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Chromium.app");
        let macos = app.join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(app.join("Contents").join("Info.plist"), INFO_PLIST).unwrap();
        let executable = macos.join("Chromium");
        std::fs::write(&executable, b"not runnable here").unwrap();

        let expected = found(148, "148.0.7778.215", ProbeMethod::InfoPlist);
        assert_eq!(probe(&executable), expected);
        assert_eq!(probe(&app), expected);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            runtime.block_on(bundle_executable(&app)),
            Some(executable.clone())
        );
        assert_eq!(runtime.block_on(bundle_executable(&executable)), None);
        // An executable name that escapes the bundle is refused.
        std::fs::write(
            app.join("Contents").join("Info.plist"),
            INFO_PLIST.replace(">Chromium<", ">../../x<"),
        )
        .unwrap();
        assert_eq!(runtime.block_on(bundle_executable(&app)), None);
    }

    #[test]
    fn missing_or_unrunnable_files_are_unknown() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(probe(&dir.path().join("missing")), KernelProbe::unknown());
        let plain = dir.path().join("chrome");
        std::fs::write(&plain, b"plain text").unwrap();
        assert_eq!(probe(&plain), KernelProbe::unknown());
    }

    #[test]
    fn serializes_for_the_bridge() {
        assert_eq!(
            serde_json::to_value(found(150, "150.0.7871.186", ProbeMethod::FileName)).unwrap(),
            serde_json::json!({
                "majorVersion": 150,
                "fullVersion": "150.0.7871.186",
                "method": "file_name",
            })
        );
        assert_eq!(
            serde_json::to_value(KernelProbe::unknown()).unwrap(),
            serde_json::json!({ "majorVersion": null, "fullVersion": null, "method": null })
        );
    }
}

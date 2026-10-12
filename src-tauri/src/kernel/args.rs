//! Launch command construction (design §3): turns a profile into the argv for
//! fingerprint-chromium, gated by what the bound kernel's major version accepts.
//!
//! Everything here is pure (no IO, no global state), so the argument contract — A3 / A4 — is
//! proven by the unit tests below. Flags and version ranges follow
//! research/fingerprint-chromium-cli.md §1–§2.

use std::fmt::Display;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::shell::{self, HostPlatform};
use crate::db::kernels::Kernel;
use crate::db::profiles::{ProfileDraft, spoofing_csv};
use crate::db::proxies::Proxy;
use crate::error::{AppError, AppResult};

/// Inclusive range of kernel major versions that accept an argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MajorRange {
    pub min: u32,
    /// `None` = still accepted by the newest kernel.
    pub max: Option<u32>,
}

impl MajorRange {
    const fn since(min: u32) -> Self {
        Self { min, max: None }
    }

    const fn between(min: u32, max: u32) -> Self {
        Self {
            min,
            max: Some(max),
        }
    }

    /// Why `major` does not accept the argument; `None` when it does.
    fn unsupported_reason(self, major: u32) -> Option<WarningReason> {
        if major < self.min {
            Some(WarningReason::RequiresNewerKernel)
        } else if self.max.is_some_and(|max| major > max) {
            Some(WarningReason::RemovedInKernel)
        } else {
            None
        }
    }

    fn supports(self, major: u32) -> bool {
        self.unsupported_reason(major).is_none()
    }
}

// The only version-dependent arguments. Everything else is accepted by every supported kernel.

/// `--fingerprint-brand`, `--fingerprint-brand-version`: added in 131.
const BRAND: MajorRange = MajorRange::since(131);
/// `--fingerprint-gpu-vendor`, `--fingerprint-gpu-renderer`: added in 139, removed in 144.
const GPU_VENDOR_RENDERER: MajorRange = MajorRange::between(139, 143);
/// `--disable-gpu-fingerprint`: added in 142, removed in 144.
const DISABLE_GPU_FINGERPRINT: MajorRange = MajorRange::between(142, 143);
/// `--disable-spoofing`: added in 144.
const DISABLE_SPOOFING: MajorRange = MajorRange::since(144);

/// Which version-dependent arguments a kernel accepts. Returned by `getArgSupport` so the
/// editor shows exactly the fields the builder will emit; the frontend never compares version
/// numbers itself (design §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ArgSupport {
    /// `--fingerprint-brand`, `--fingerprint-brand-version` (131+).
    pub brand: bool,
    /// `--fingerprint-gpu-vendor`, `--fingerprint-gpu-renderer` (139–143).
    pub gpu_vendor_renderer: bool,
    /// `--disable-gpu-fingerprint` (142–143).
    pub disable_gpu_fingerprint: bool,
    /// `--disable-spoofing` (144+).
    pub disable_spoofing: bool,
}

impl ArgSupport {
    pub fn for_major(major: u32) -> Self {
        Self {
            brand: BRAND.supports(major),
            gpu_vendor_renderer: GPU_VENDOR_RENDERER.supports(major),
            disable_gpu_fingerprint: DISABLE_GPU_FINGERPRINT.supports(major),
            disable_spoofing: DISABLE_SPOOFING.supports(major),
        }
    }
}

/// The major version whose argument set applies to a profile: its kernel's, or the newest set
/// when no kernel is configured (design §3.2).
pub fn effective_major(kernel: Option<&Kernel>) -> u32 {
    kernel.map_or(u32::MAX, |kernel| kernel.major_version)
}

/// What to spawn, plus its display form. Returned to the editor by `previewCommand`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LaunchPlan {
    pub program: PathBuf,
    /// Passed to the process as-is: no shell, so no quotes.
    pub args: Vec<String>,
    /// Copyable command line, quoted for the host's shell. Display only.
    pub shell_line: String,
}

/// Builds the command line of a profile (design §3.3).
///
/// `draft` may be an unsaved editor draft: blank text counts as unset and values are trimmed,
/// exactly as saving would store them. `user_data_dir` is resolved by the caller (a placeholder
/// for a profile that has no id yet). Arguments the kernel does not accept are left out; use
/// [`validate_profile_for_kernel`] to tell the user about them.
///
/// Fails with `invalid_input` when the extra arguments do not pass [`split_extra_args`], when a
/// start URL sets the user data directory ([`check_start_urls`]), or when `user_data_dir` is
/// not valid UTF-8.
pub fn build_launch_plan(
    draft: &ProfileDraft,
    kernel: &Kernel,
    proxy: Option<&Proxy>,
    user_data_dir: &Path,
    host: HostPlatform,
) -> AppResult<LaunchPlan> {
    let support = ArgSupport::for_major(kernel.major_version);
    let user_data_dir = user_data_dir.to_str().ok_or_else(|| {
        AppError::invalid_input(format!(
            "user data directory is not valid UTF-8: {}",
            user_data_dir.display()
        ))
    })?;

    let mut args = vec![format!("--user-data-dir={user_data_dir}")];
    // Without a seed fingerprint-chromium behaves like stock Chromium, so every fingerprint
    // option would be inert.
    if let Some(seed) = draft.fingerprint_seed {
        push_value(&mut args, "fingerprint", Some(seed));
        push_value(&mut args, "fingerprint-platform", draft.fp_platform);
        push_value(
            &mut args,
            "fingerprint-platform-version",
            non_blank(&draft.fp_platform_version),
        );
        if support.brand {
            push_value(&mut args, "fingerprint-brand", non_blank(&draft.fp_brand));
            push_value(
                &mut args,
                "fingerprint-brand-version",
                non_blank(&draft.fp_brand_version),
            );
        }
        push_value(
            &mut args,
            "fingerprint-hardware-concurrency",
            draft.fp_hardware_concurrency,
        );
        if support.gpu_vendor_renderer {
            push_value(
                &mut args,
                "fingerprint-gpu-vendor",
                non_blank(&draft.fp_gpu_vendor),
            );
            push_value(
                &mut args,
                "fingerprint-gpu-renderer",
                non_blank(&draft.fp_gpu_renderer),
            );
        }
        push_flag(
            &mut args,
            "disable-gpu-fingerprint",
            support.disable_gpu_fingerprint && draft.disable_gpu_fingerprint,
        );
        if support.disable_spoofing {
            push_value(
                &mut args,
                "disable-spoofing",
                spoofing_csv(&draft.disable_spoofing),
            );
        }
    }
    push_flag(
        &mut args,
        "disable-non-proxied-udp",
        draft.disable_non_proxied_udp,
    );
    push_value(&mut args, "lang", non_blank(&draft.lang));
    push_value(&mut args, "accept-lang", non_blank(&draft.accept_lang));
    push_value(&mut args, "timezone", non_blank(&draft.timezone));
    push_value(&mut args, "proxy-server", proxy.map(Proxy::url));
    push_flag(&mut args, "headless", draft.headless);
    // After the generated arguments, so the user has the last word (Chromium keeps the last
    // occurrence of a switch).
    if let Some(extra) = non_blank(&draft.extra_args) {
        args.extend(split_extra_args(extra)?);
    }
    let start_urls: Vec<String> = draft
        .start_urls
        .iter()
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .collect();
    check_start_urls(&start_urls)?;
    args.extend(start_urls);

    Ok(LaunchPlan {
        shell_line: shell::command_line(&kernel.exec_path, &args, host),
        program: PathBuf::from(&kernel.exec_path),
        args,
    })
}

/// Splits the free-text extra arguments into words. The rules are the same on every OS:
///
/// - Whitespace separates words: spaces, tabs, line breaks, and also Unicode spaces such as
///   the no-break space and the full-width space an IME types (the same set `trim` removes).
/// - `"..."` and `'...'` keep whitespace inside one word. The quotes are removed, and the
///   quoted text joins whatever touches it: `--name="My Shop"` is `--name=My Shop`.
/// - Inside one kind of quote the other kind is an ordinary character, so a quote character is
///   written inside the other kind: `'say "hi"'`, `"it's"`.
/// - Backslashes are ordinary characters, so Windows paths are written as they are:
///   `--load-extension=C:\ext`, `"C:\Program Files\ext"`.
/// - Nothing else is special: no comments, variables or wildcards (`#`, `$`, `~`, `*`).
/// - `""` or `''` standing alone is an empty word.
/// - Typographic quotes (`“ ” ‘ ’ ＂ ＇`) are rejected outside straight quotes, so an IME can't
///   silently split a value; inside straight quotes they are ordinary characters.
///
/// Fails with `invalid_input` when a quote is left open, when the text contains NUL (no OS can
/// pass it in an argument), or when a word sets `--user-data-dir`: the profile has its own
/// field for that, and the launcher has to know the real directory (single-instance check,
/// design §4.2). Saving a profile runs this, so a stored profile always splits.
pub fn split_extra_args(text: &str) -> AppResult<Vec<String>> {
    if text.contains('\0') {
        return Err(AppError::invalid_input(
            "extra arguments must not contain NUL characters",
        ));
    }
    let words = split_words(text)?;
    reject_user_data_dir(&words, "extra arguments")?;
    Ok(words)
}

/// Start URLs are positional, but Chromium takes any argument that looks like a switch for
/// one, wherever it stands, so a start "URL" must not set `--user-data-dir` either. Expects
/// trimmed URLs; saving a profile runs this too.
pub fn check_start_urls(urls: &[String]) -> AppResult<()> {
    reject_user_data_dir(urls, "start URLs")
}

fn reject_user_data_dir(words: &[String], field: &str) -> AppResult<()> {
    match words.iter().find(|word| sets_user_data_dir(word)) {
        Some(word) => Err(AppError::invalid_input(format!(
            "{field} must not set the user data directory ({word}); \
             use the profile's user data directory field"
        ))),
        None => Ok(()),
    }
}

/// The splitting rules of [`split_extra_args`].
fn split_words(text: &str) -> AppResult<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    // A word starts at its first character or quote, so a lone `""` is an (empty) word.
    let mut in_word = false;
    let mut open_quote = None;
    for c in text.chars() {
        match (open_quote, c) {
            (Some(quote), c) if c == quote => open_quote = None,
            (Some(_), c) => word.push(c),
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, '"' | '\'') => {
                open_quote = Some(c);
                in_word = true;
            }
            // Typographic quotes an IME types instead of `"` / `'` would silently split
            // `--name=“My Shop”` into two words.
            (None, '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}' | '\u{FF02}' | '\u{FF07}') => {
                return Err(AppError::invalid_input(format!(
                    "extra arguments: {c} is not a quote here; use straight quotes \" or '"
                )));
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if let Some(quote) = open_quote {
        return Err(AppError::invalid_input(format!(
            "extra arguments: missing closing {quote}"
        )));
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// `--user-data-dir` / `--user-data-dir=...` in any letter case, also with the `-` prefix
/// Chromium accepts everywhere and the `/` prefix it accepts on Windows. Profiles move between
/// OSes, so all spellings are rejected on every OS.
fn sets_user_data_dir(word: &str) -> bool {
    let name = word.split_once('=').map_or(word, |(name, _)| name);
    ["--", "-", "/"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|name| name.eq_ignore_ascii_case("user-data-dir"))
    })
}

/// A draft field whose argument only some kernel versions accept. Serialized as the field's
/// name in the bridge `ProfileDraft` (`fpGpuVendor`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GatedField {
    FpBrand,
    FpBrandVersion,
    FpGpuVendor,
    FpGpuRenderer,
    DisableGpuFingerprint,
    DisableSpoofing,
}

/// Machine-readable cause of a [`FieldWarning`]; the frontend picks the i18n text by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum WarningReason {
    /// The kernel predates the argument.
    RequiresNewerKernel,
    /// The argument was removed before the kernel's version.
    RemovedInKernel,
}

/// A field that is set but will not be passed to the kernel, because the kernel's version
/// does not accept its argument. The value itself is kept (R2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FieldWarning {
    pub field: GatedField,
    pub reason: WarningReason,
    /// Versions that do accept the argument, for the message text.
    pub supported: MajorRange,
}

/// Warnings for every set field that a kernel of `kernel_major` would ignore (design §3.2).
/// Pass [`effective_major`] of the profile's kernel. Unset and blank fields never warn.
pub fn validate_profile_for_kernel(draft: &ProfileDraft, kernel_major: u32) -> Vec<FieldWarning> {
    let gated = [
        (
            GatedField::FpBrand,
            BRAND,
            non_blank(&draft.fp_brand).is_some(),
        ),
        (
            GatedField::FpBrandVersion,
            BRAND,
            non_blank(&draft.fp_brand_version).is_some(),
        ),
        (
            GatedField::FpGpuVendor,
            GPU_VENDOR_RENDERER,
            non_blank(&draft.fp_gpu_vendor).is_some(),
        ),
        (
            GatedField::FpGpuRenderer,
            GPU_VENDOR_RENDERER,
            non_blank(&draft.fp_gpu_renderer).is_some(),
        ),
        (
            GatedField::DisableGpuFingerprint,
            DISABLE_GPU_FINGERPRINT,
            draft.disable_gpu_fingerprint,
        ),
        (
            GatedField::DisableSpoofing,
            DISABLE_SPOOFING,
            !draft.disable_spoofing.is_empty(),
        ),
    ];
    gated
        .into_iter()
        .filter(|&(_, _, is_set)| is_set)
        .filter_map(|(field, supported, _)| {
            supported
                .unsupported_reason(kernel_major)
                .map(|reason| FieldWarning {
                    field,
                    reason,
                    supported,
                })
        })
        .collect()
}

/// Trimmed text, `None` when unset or blank.
fn non_blank(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// `--name=value` (never quoted: argv goes to the process without a shell).
fn push_value(args: &mut Vec<String>, name: &str, value: Option<impl Display>) {
    if let Some(value) = value {
        args.push(format!("--{name}={value}"));
    }
}

fn push_flag(args: &mut Vec<String>, name: &str, on: bool) {
    if on {
        args.push(format!("--{name}"));
    }
}

/// Paths in these tests are plain literals on every OS: the builder never touches the
/// filesystem and converts paths with `Path::to_str`, which returns the string unchanged on
/// Windows too, so the same assertions hold on every CI runner.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profiles::{FpPlatform, SpoofingFeature};
    use crate::db::proxies::ProxyScheme;
    use crate::error::ErrorCode;
    use crate::kernel::shell::tests::{cmd_split, windows_split};

    /// Every kernel major the version-gating tests run against (design §3.5).
    const MAJORS: [u32; 9] = [130, 131, 138, 139, 141, 142, 143, 144, 150];

    const UDD: &str = "/data/profiles/p1";

    fn kernel(major: u32, exec_path: &str) -> Kernel {
        Kernel {
            id: "kernel".into(),
            name: format!("Chromium {major}"),
            exec_path: exec_path.into(),
            major_version: major,
            full_version: None,
            is_default: true,
            created_at: String::new(),
        }
    }

    fn proxy(scheme: ProxyScheme, host: &str, port: u16) -> Proxy {
        Proxy {
            id: "proxy".into(),
            name: None,
            scheme,
            host: host.into(),
            port,
            note: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn build(draft: &ProfileDraft, major: u32, proxy: Option<&Proxy>) -> AppResult<LaunchPlan> {
        build_launch_plan(
            draft,
            &kernel(major, "/opt/chromium/chrome"),
            proxy,
            Path::new(UDD),
            HostPlatform::Unix,
        )
    }

    fn argv(draft: &ProfileDraft, major: u32, proxy: Option<&Proxy>) -> Vec<String> {
        build(draft, major, proxy).unwrap().args
    }

    fn sorted(mut words: Vec<String>) -> Vec<String> {
        words.sort();
        words
    }

    /// Every field set, including all version-dependent ones.
    fn full_draft() -> ProfileDraft {
        ProfileDraft {
            name: "Shop".into(),
            fingerprint_seed: Some(1234),
            fp_platform: Some(FpPlatform::Windows),
            fp_platform_version: Some("10.0.0".into()),
            fp_brand: Some("Chrome".into()),
            fp_brand_version: Some("150.0.7871.186".into()),
            fp_hardware_concurrency: Some(8),
            fp_gpu_vendor: Some("NVIDIA Corporation".into()),
            fp_gpu_renderer: Some("NVIDIA GeForce GTX 1060".into()),
            disable_gpu_fingerprint: true,
            disable_spoofing: vec![SpoofingFeature::Gpu, SpoofingFeature::Font],
            disable_non_proxied_udp: true,
            lang: Some("zh-CN".into()),
            accept_lang: Some("zh-CN,en-US".into()),
            timezone: Some("Asia/Shanghai".into()),
            headless: true,
            start_urls: vec!["https://example.com".into()],
            extra_args: Some("--no-first-run".into()),
            ..ProfileDraft::default()
        }
    }

    fn full_proxy() -> Proxy {
        proxy(ProxyScheme::Socks5, "127.0.0.1", 1080)
    }

    /// README "基本使用":
    /// `./chrome --fingerprint=1000 --user-data-dir=/tmp/chromium/1000
    ///  --timezone="America/Los_Angeles" --proxy-server="你的代理服务器地址"`
    #[test]
    fn readme_example_basic_usage() {
        let draft = ProfileDraft {
            fingerprint_seed: Some(1000),
            timezone: Some("America/Los_Angeles".into()),
            // The README line has no WebRTC flag; the launcher's default would add one.
            disable_non_proxied_udp: false,
            ..ProfileDraft::default()
        };
        // Stands in for the README's "你的代理服务器地址" placeholder.
        let proxy = proxy(ProxyScheme::Http, "127.0.0.1", 8080);
        // The README line with its shell quotes removed (our splitting rules agree with `sh`
        // on this line).
        let readme = split_words(
            r#"--fingerprint=1000 --user-data-dir=/tmp/chromium/1000 --timezone="America/Los_Angeles" --proxy-server="http://127.0.0.1:8080""#,
        )
        .unwrap();
        assert_eq!(readme[2], "--timezone=America/Los_Angeles");

        // Every argument here exists on every kernel, so the result is the same on all of them.
        for major in MAJORS {
            let plan = build_launch_plan(
                &draft,
                &kernel(major, "./chrome"),
                Some(&proxy),
                Path::new("/tmp/chromium/1000"),
                HostPlatform::Unix,
            )
            .unwrap();
            assert_eq!(plan.program, PathBuf::from("./chrome"));
            // No shell quotes in argv: the process receives the bare values.
            assert_eq!(
                plan.args,
                [
                    "--user-data-dir=/tmp/chromium/1000",
                    "--fingerprint=1000",
                    "--timezone=America/Los_Angeles",
                    "--proxy-server=http://127.0.0.1:8080",
                ],
                "major {major}"
            );
            assert_eq!(
                plan.shell_line,
                "./chrome --user-data-dir=/tmp/chromium/1000 --fingerprint=1000 \
                 --timezone=America/Los_Angeles --proxy-server=http://127.0.0.1:8080"
            );
            // The README's argument set exactly; only the order is ours (design §3.3).
            assert_eq!(sorted(plan.args), sorted(readme.clone()));
        }
    }

    /// README "定制 User-Agent":
    /// `chrome.exe --fingerprint=2023 --fingerprint-platform=macos
    ///  --fingerprint-platform-version="15.2.0" --fingerprint-brand="Edge"
    ///  --user-data-dir=%TEMP%\chromium`
    ///
    /// `%TEMP%` is expanded by cmd.exe, and the launcher spawns without a shell, so the fixture
    /// uses a concrete absolute path in its place.
    #[test]
    fn readme_example_custom_user_agent() {
        let draft = ProfileDraft {
            fingerprint_seed: Some(2023),
            fp_platform: Some(FpPlatform::Macos),
            fp_platform_version: Some("15.2.0".into()),
            fp_brand: Some("Edge".into()),
            disable_non_proxied_udp: false,
            ..ProfileDraft::default()
        };
        let udd = r"C:\Users\me\AppData\Local\Temp\chromium";
        let readme = windows_split(&format!(
            r#"--fingerprint=2023 --fingerprint-platform=macos --fingerprint-platform-version="15.2.0" --fingerprint-brand="Edge"  --user-data-dir={udd}"#
        ));

        // --fingerprint-brand needs 131+.
        for major in MAJORS.into_iter().filter(|&m| m >= 131) {
            let plan = build_launch_plan(
                &draft,
                &kernel(major, "chrome.exe"),
                None,
                Path::new(udd),
                HostPlatform::Windows,
            )
            .unwrap();
            assert_eq!(plan.program, PathBuf::from("chrome.exe"));
            assert_eq!(
                plan.args,
                [
                    r"--user-data-dir=C:\Users\me\AppData\Local\Temp\chromium",
                    "--fingerprint=2023",
                    "--fingerprint-platform=macos",
                    "--fingerprint-platform-version=15.2.0",
                    "--fingerprint-brand=Edge",
                ],
                "major {major}"
            );
            assert_eq!(
                plan.shell_line,
                r"chrome.exe --user-data-dir=C:\Users\me\AppData\Local\Temp\chromium --fingerprint=2023 --fingerprint-platform=macos --fingerprint-platform-version=15.2.0 --fingerprint-brand=Edge"
            );
            assert_eq!(cmd_split(&plan.shell_line)[1..], plan.args);
            assert_eq!(sorted(plan.args), sorted(readme.clone()));
        }
    }

    #[test]
    fn arg_support_boundaries() {
        // (major, brand, gpu vendor/renderer, disable-gpu-fingerprint, disable-spoofing)
        let expected = [
            (130, false, false, false, false),
            (131, true, false, false, false),
            (138, true, false, false, false),
            (139, true, true, false, false),
            (141, true, true, false, false),
            (142, true, true, true, false),
            (143, true, true, true, false),
            (144, true, false, false, true),
            (150, true, false, false, true),
        ];
        assert_eq!(expected.map(|row| row.0), MAJORS);
        for (major, brand, gpu_vendor_renderer, disable_gpu_fingerprint, disable_spoofing) in
            expected
        {
            assert_eq!(
                ArgSupport::for_major(major),
                ArgSupport {
                    brand,
                    gpu_vendor_renderer,
                    disable_gpu_fingerprint,
                    disable_spoofing,
                },
                "major {major}"
            );
        }
    }

    /// The full argv of [`full_draft`] on each kernel major (A3 / A4).
    #[test]
    fn version_gating_full_argument_set() {
        let brand = [
            "--fingerprint-brand=Chrome",
            "--fingerprint-brand-version=150.0.7871.186",
        ];
        let gpu = [
            "--fingerprint-gpu-vendor=NVIDIA Corporation",
            "--fingerprint-gpu-renderer=NVIDIA GeForce GTX 1060",
        ];
        let hwc = "--fingerprint-hardware-concurrency=8";
        let disable_gpu = "--disable-gpu-fingerprint";
        let spoofing = "--disable-spoofing=font,gpu";
        // Arguments between --fingerprint-platform-version and --disable-non-proxied-udp.
        let gated: [(u32, Vec<&str>); 9] = [
            (130, vec![hwc]),
            (131, vec![brand[0], brand[1], hwc]),
            (138, vec![brand[0], brand[1], hwc]),
            (139, vec![brand[0], brand[1], hwc, gpu[0], gpu[1]]),
            (141, vec![brand[0], brand[1], hwc, gpu[0], gpu[1]]),
            (
                142,
                vec![brand[0], brand[1], hwc, gpu[0], gpu[1], disable_gpu],
            ),
            (
                143,
                vec![brand[0], brand[1], hwc, gpu[0], gpu[1], disable_gpu],
            ),
            (144, vec![brand[0], brand[1], hwc, spoofing]),
            (150, vec![brand[0], brand[1], hwc, spoofing]),
        ];
        assert_eq!(gated.each_ref().map(|row| row.0), MAJORS);

        for (major, middle) in gated {
            let mut expected = vec![
                "--user-data-dir=/data/profiles/p1",
                "--fingerprint=1234",
                "--fingerprint-platform=windows",
                "--fingerprint-platform-version=10.0.0",
            ];
            expected.extend(middle);
            expected.extend([
                "--disable-non-proxied-udp",
                "--lang=zh-CN",
                "--accept-lang=zh-CN,en-US",
                "--timezone=Asia/Shanghai",
                "--proxy-server=socks5://127.0.0.1:1080",
                "--headless",
                "--no-first-run",
                "https://example.com",
            ]);
            assert_eq!(
                argv(&full_draft(), major, Some(&full_proxy())),
                expected,
                "major {major}"
            );
        }
    }

    #[test]
    fn no_seed_means_no_fingerprint_arguments() {
        let draft = ProfileDraft {
            fingerprint_seed: None,
            ..full_draft()
        };
        for major in MAJORS {
            assert_eq!(
                argv(&draft, major, Some(&full_proxy())),
                [
                    "--user-data-dir=/data/profiles/p1",
                    "--disable-non-proxied-udp",
                    "--lang=zh-CN",
                    "--accept-lang=zh-CN,en-US",
                    "--timezone=Asia/Shanghai",
                    "--proxy-server=socks5://127.0.0.1:1080",
                    "--headless",
                    "--no-first-run",
                    "https://example.com",
                ],
                "major {major}"
            );
        }
    }

    #[test]
    fn empty_draft() {
        for major in MAJORS {
            // WebRTC leak protection is the only default-on argument.
            assert_eq!(
                argv(&ProfileDraft::default(), major, None),
                [
                    "--user-data-dir=/data/profiles/p1",
                    "--disable-non-proxied-udp"
                ]
            );
            let bare = ProfileDraft {
                disable_non_proxied_udp: false,
                ..ProfileDraft::default()
            };
            assert_eq!(
                argv(&bare, major, None),
                ["--user-data-dir=/data/profiles/p1"]
            );
        }
    }

    #[test]
    fn blank_text_is_unset_and_values_are_trimmed() {
        let blank = || Some(" \t ".to_owned());
        let draft = ProfileDraft {
            fingerprint_seed: Some(1),
            fp_platform_version: blank(),
            fp_brand: blank(),
            fp_brand_version: blank(),
            fp_gpu_vendor: blank(),
            fp_gpu_renderer: blank(),
            lang: Some(String::new()),
            accept_lang: blank(),
            timezone: blank(),
            extra_args: Some(" \n ".into()),
            start_urls: vec![String::new(), "  ".into()],
            disable_non_proxied_udp: false,
            ..ProfileDraft::default()
        };
        for major in MAJORS {
            assert_eq!(
                argv(&draft, major, None),
                ["--user-data-dir=/data/profiles/p1", "--fingerprint=1"],
                "major {major}"
            );
        }

        let padded = ProfileDraft {
            fingerprint_seed: Some(1),
            fp_brand: Some(" Edge ".into()),
            timezone: Some(" UTC ".into()),
            start_urls: vec![" https://a.test ".into()],
            disable_non_proxied_udp: false,
            ..ProfileDraft::default()
        };
        assert_eq!(
            argv(&padded, 150, None),
            [
                "--user-data-dir=/data/profiles/p1",
                "--fingerprint=1",
                "--fingerprint-brand=Edge",
                "--timezone=UTC",
                "https://a.test",
            ]
        );
    }

    #[test]
    fn disable_spoofing_is_canonical_and_omitted_when_empty() {
        use SpoofingFeature::*;

        let with = |features: Vec<SpoofingFeature>| ProfileDraft {
            fingerprint_seed: Some(1),
            disable_spoofing: features,
            disable_non_proxied_udp: false,
            ..ProfileDraft::default()
        };
        assert_eq!(
            argv(&with(vec![Gpu, Font, Gpu, ClientRects]), 144, None)[2],
            "--disable-spoofing=font,clientrects,gpu"
        );
        assert_eq!(
            argv(
                &with(vec![Gpu, ClientRects, Canvas, Audio, Font]),
                150,
                None
            )[2],
            "--disable-spoofing=font,audio,canvas,clientrects,gpu"
        );
        assert_eq!(argv(&with(vec![]), 144, None).len(), 2);
    }

    #[test]
    fn extra_args_are_split_and_appended_before_start_urls() {
        let draft = ProfileDraft {
            extra_args: Some(
                r#"--window-size=1280,800 --window-name="My Shop" '--x=a b' --empty="" --load-extension=C:\ext --dq="C:\Program Files\y\""#
                    .into(),
            ),
            start_urls: vec!["https://a.test".into()],
            disable_non_proxied_udp: false,
            ..ProfileDraft::default()
        };
        let plan = build(&draft, 150, None).unwrap();
        assert_eq!(
            plan.args,
            [
                "--user-data-dir=/data/profiles/p1",
                "--window-size=1280,800",
                "--window-name=My Shop",
                "--x=a b",
                "--empty=",
                r"--load-extension=C:\ext",
                r"--dq=C:\Program Files\y\",
                "https://a.test",
            ]
        );
        assert_eq!(
            plan.shell_line,
            r"/opt/chromium/chrome --user-data-dir=/data/profiles/p1 --window-size=1280,800 --window-name='My Shop' --x='a b' --empty= --load-extension='C:\ext' --dq='C:\Program Files\y\' https://a.test"
        );
    }

    #[test]
    fn typographic_quotes_are_rejected_outside_straight_quotes() {
        for bad in [
            "--window-name=“My Shop”",
            "--x=‘a b’",
            "--x=＂a＂",
            "--x=＇a＇",
            "--a ”",
        ] {
            let err = split_extra_args(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
            assert!(
                err.message.contains("straight quotes"),
                "{bad}: {}",
                err.message
            );
        }
        // Inside straight quotes they are ordinary characters.
        assert_eq!(
            split_extra_args(r#"--window-name="My “Shop”" '‘x’'"#).unwrap(),
            ["--window-name=My “Shop”", "‘x’"]
        );
    }

    #[test]
    fn split_extra_args_rules() {
        let split = |text: &str| split_extra_args(text).unwrap();

        // Spaces, tabs, CR and LF separate; runs of them count once.
        assert_eq!(
            split(" --a\t--b\r\n--c\r--d\n\n --e  "),
            ["--a", "--b", "--c", "--d", "--e"]
        );
        assert!(split("").is_empty());
        assert!(split(" \t\r\n ").is_empty());
        // Unicode whitespace separates too, as `trim` strips it: the no-break space, and the
        // full-width space an IME types. Inside quotes it is kept.
        assert_eq!(split("--a\u{3000}--b\u{a0}--c"), ["--a", "--b", "--c"]);
        assert!(split("\u{3000}\u{a0}").is_empty());
        assert_eq!(split("'a\u{3000}b'"), ["a\u{3000}b"]);

        // Quotes group, are removed, and join adjacent text.
        assert_eq!(split(r#"--x="a b""#), ["--x=a b"]);
        assert_eq!(split(r#"a"b c"d"#), ["ab cd"]);
        assert_eq!(split("'--x=a b'"), ["--x=a b"]);
        assert_eq!(split(r#""a"'b'c"#), ["abc"]);
        assert_eq!(
            split("\"line\none\" 'tab\there'"),
            ["line\none", "tab\there"]
        );
        // The other kind of quote is literal inside quotes.
        assert_eq!(split(r#"--n='say "hi"'"#), [r#"--n=say "hi""#]);
        assert_eq!(split(r#"--n="it's""#), ["--n=it's"]);
        assert_eq!(split(r#"--n="it's "'"mine"'"#), [r#"--n=it's "mine""#]);

        // Backslashes are always literal: Windows paths survive with and without spaces.
        assert_eq!(
            split(r"--load-extension=C:\ext --disk-cache-dir=D:\cache\"),
            [r"--load-extension=C:\ext", r"--disk-cache-dir=D:\cache\"]
        );
        assert_eq!(
            split(r#""C:\Program Files\x" --dir="C:\My Data\" next"#),
            [r"C:\Program Files\x", r"--dir=C:\My Data\", "next"]
        );
        assert_eq!(
            split(r"\\server\share a\ b"),
            [r"\\server\share", r"a\", "b"]
        );
        assert_eq!(split(r#"a\"b c""#), [r"a\b c"]);

        // No comments, variables or wildcards.
        assert_eq!(
            split("--a # note https://a.test/#x $HOME ~ *.txt"),
            [
                "--a",
                "#",
                "note",
                "https://a.test/#x",
                "$HOME",
                "~",
                "*.txt"
            ]
        );

        // Empty quotes: an empty word when they stand alone, nothing when they touch text.
        assert_eq!(split(r#""""#), [""]);
        assert_eq!(split(r#"'' x """#), ["", "x", ""]);
        assert_eq!(split(r#"--x="" a""b ''c"#), ["--x=", "ab", "c"]);

        // An open quote is an error, never a panic.
        for (bad, quote) in [
            (r#"--a="b"#, '"'),
            ("'x", '\''),
            (r#""a'"#, '"'),
            (r#"--a='b""#, '\''),
            (r#"ok "x" "y"#, '"'),
        ] {
            let err = split_extra_args(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
            assert_eq!(
                err.message,
                format!("extra arguments: missing closing {quote}"),
                "{bad}"
            );
        }
        let draft = ProfileDraft {
            extra_args: Some("--no-sandbox '--x".into()),
            ..ProfileDraft::default()
        };
        assert_eq!(
            build(&draft, 150, None).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        // No OS can pass NUL inside an argument; say so on save rather than at spawn.
        for bad in ["--a=\0", "'\0'"] {
            let err = split_extra_args(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
        }
    }

    #[test]
    fn extra_args_must_not_set_the_user_data_dir() {
        for bad in [
            "--user-data-dir=/elsewhere",
            "--user-data-dir",
            "--user-data-dir=",
            r"--USER-Data-Dir=C:\elsewhere",
            r#"--no-first-run "--user-data-dir=/my dir""#,
            "--user-data-dir''=/x",
            "-user-data-dir=/x",
            r"/user-data-dir=C:\x",
        ] {
            let err = split_extra_args(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
            assert!(
                err.message.contains("user data directory field"),
                "{}",
                err.message
            );
            let draft = ProfileDraft {
                extra_args: Some(bad.into()),
                ..ProfileDraft::default()
            };
            assert_eq!(
                build(&draft, 150, None).unwrap_err().code,
                ErrorCode::InvalidInput,
                "{bad}"
            );
        }
        // Chromium parses switches among the positional arguments too, so a start "URL" is
        // checked the same way (it is not split: each URL is one argument).
        for bad in [
            "--user-data-dir=/elsewhere",
            r"--USER-DATA-DIR=C:\x",
            "-user-data-dir",
            r"/user-data-dir=C:\x",
        ] {
            let draft = ProfileDraft {
                start_urls: vec!["https://a.test".into(), format!(" {bad} ")],
                ..ProfileDraft::default()
            };
            let err = build(&draft, 150, None).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
            assert!(err.message.starts_with("start URLs"), "{}", err.message);
        }
        assert!(
            check_start_urls(&[
                "https://a.test/?--user-data-dir=x".into(),
                "/home/me/user-data-dir/start.html".into(),
            ])
            .is_ok()
        );
        // Other switches and values that merely mention it are fine.
        assert_eq!(
            split_extra_args(
                "--user-data-directory=x ---user-data-dir=x --disk-cache-dir=/user-data-dir \
                 https://a.test/?--user-data-dir=x"
            )
            .unwrap()
            .len(),
            4
        );
    }

    #[test]
    fn proxy_server_argument() {
        let cases = [
            (
                ProxyScheme::Http,
                "127.0.0.1",
                8080,
                "http://127.0.0.1:8080",
            ),
            (
                ProxyScheme::Socks5,
                "proxy.example.com",
                1080,
                "socks5://proxy.example.com:1080",
            ),
            (ProxyScheme::Socks5, "::1", 1080, "socks5://[::1]:1080"),
            (
                ProxyScheme::Http,
                "2001:db8::1",
                3128,
                "http://[2001:db8::1]:3128",
            ),
        ];
        let draft = ProfileDraft {
            disable_non_proxied_udp: false,
            ..ProfileDraft::default()
        };
        for (scheme, host, port, url) in cases {
            let proxy = proxy(scheme, host, port);
            assert_eq!(
                argv(&draft, 150, Some(&proxy)),
                [
                    "--user-data-dir=/data/profiles/p1".to_owned(),
                    format!("--proxy-server={url}")
                ]
            );
        }
    }

    #[test]
    fn start_urls_come_last_in_order() {
        let draft = ProfileDraft {
            start_urls: vec![
                "https://a.test".into(),
                " ".into(),
                "https://b.test/?q=1&x=2".into(),
            ],
            extra_args: Some("--no-first-run".into()),
            headless: true,
            ..ProfileDraft::default()
        };
        let plan = build(&draft, 150, None).unwrap();
        assert_eq!(
            plan.args,
            [
                "--user-data-dir=/data/profiles/p1",
                "--disable-non-proxied-udp",
                "--headless",
                "--no-first-run",
                "https://a.test",
                "https://b.test/?q=1&x=2",
            ]
        );
        assert!(
            plan.shell_line
                .ends_with(" https://a.test 'https://b.test/?q=1&x=2'"),
            "{}",
            plan.shell_line
        );
    }

    /// Paths with spaces and a value with both kinds of quote.
    fn quoting_draft() -> ProfileDraft {
        ProfileDraft {
            lang: Some("zh-CN".into()),
            extra_args: Some(r#"--window-name="it's "'"mine"'"#.into()),
            disable_non_proxied_udp: false,
            ..ProfileDraft::default()
        }
    }

    fn unix_quoting_plan() -> LaunchPlan {
        build_launch_plan(
            &quoting_draft(),
            &kernel(150, "/opt/My Apps/chrome"),
            None,
            Path::new("/home/me/My Profiles/p1"),
            HostPlatform::Unix,
        )
        .unwrap()
    }

    #[test]
    fn shell_line_follows_the_host() {
        let unix = unix_quoting_plan();
        assert_eq!(unix.args[2], r#"--window-name=it's "mine""#);
        assert_eq!(
            unix.shell_line,
            r#"'/opt/My Apps/chrome' --user-data-dir='/home/me/My Profiles/p1' --lang=zh-CN --window-name='it'\''s "mine"'"#
        );

        let windows = build_launch_plan(
            &quoting_draft(),
            &kernel(150, r"C:\Program Files\Chromium\chrome.exe"),
            None,
            Path::new(r"C:\Users\me\My Profiles\p1\"),
            HostPlatform::Windows,
        )
        .unwrap();
        assert_eq!(
            windows.shell_line,
            r#""C:\Program Files\Chromium\chrome.exe" --user-data-dir="C:\Users\me\My Profiles\p1\\" --lang=zh-CN --window-name="it's \"mine\"""#
        );
        assert_eq!(cmd_split(&windows.shell_line)[1..], windows.args);
        // argv itself is identical for both hosts apart from the paths.
        assert_eq!(unix.args[1..], windows.args[1..]);
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_line_round_trips_through_sh() {
        let plan = unix_quoting_plan();
        let words = crate::kernel::shell::tests::sh_split(&plan.shell_line);
        assert_eq!(words[0], "/opt/My Apps/chrome");
        assert_eq!(words[1..], plan.args);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_user_data_dir_is_rejected() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let err = build_launch_plan(
            &ProfileDraft::default(),
            &kernel(150, "/opt/chromium/chrome"),
            None,
            Path::new(OsStr::from_bytes(b"/data/\xff")),
            HostPlatform::Unix,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn effective_major_falls_back_to_the_newest_argument_set() {
        assert_eq!(effective_major(Some(&kernel(142, "/k"))), 142);
        assert_eq!(
            ArgSupport::for_major(effective_major(None)),
            ArgSupport::for_major(150)
        );
    }

    #[test]
    fn serializes_camel_case() {
        assert_eq!(
            serde_json::to_value(ArgSupport::for_major(142)).unwrap(),
            serde_json::json!({
                "brand": true,
                "gpuVendorRenderer": true,
                "disableGpuFingerprint": true,
                "disableSpoofing": false,
            })
        );
        let plan = build(&ProfileDraft::default(), 150, None).unwrap();
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            serde_json::json!({
                "program": "/opt/chromium/chrome",
                "args": ["--user-data-dir=/data/profiles/p1", "--disable-non-proxied-udp"],
                "shellLine": "/opt/chromium/chrome --user-data-dir=/data/profiles/p1 --disable-non-proxied-udp",
            })
        );
        let warning = validate_profile_for_kernel(&full_draft(), 144)[0];
        assert_eq!(
            serde_json::to_value(warning).unwrap(),
            serde_json::json!({
                "field": "fpGpuVendor",
                "reason": "removed_in_kernel",
                "supported": { "min": 139, "max": 143 },
            })
        );
    }

    #[test]
    fn gated_field_names_match_the_draft_fields() {
        let draft = serde_json::to_value(ProfileDraft::default()).unwrap();
        for field in [
            GatedField::FpBrand,
            GatedField::FpBrandVersion,
            GatedField::FpGpuVendor,
            GatedField::FpGpuRenderer,
            GatedField::DisableGpuFingerprint,
            GatedField::DisableSpoofing,
        ] {
            let name = serde_json::to_value(field).unwrap();
            let name = name.as_str().unwrap();
            assert!(
                draft.get(name).is_some(),
                "{name} is not a ProfileDraft field"
            );
        }
    }

    fn warning(field: GatedField, reason: WarningReason, supported: MajorRange) -> FieldWarning {
        FieldWarning {
            field,
            reason,
            supported,
        }
    }

    /// Switching a profile between kernels 142 and 144 (A4).
    #[test]
    fn warnings_when_switching_between_142_and_144() {
        use GatedField::*;
        use WarningReason::*;

        let made_on_142 = ProfileDraft {
            fingerprint_seed: Some(1),
            fp_gpu_vendor: Some("Intel Inc.".into()),
            fp_gpu_renderer: Some("Intel Iris OpenGL Engine".into()),
            disable_gpu_fingerprint: true,
            ..ProfileDraft::default()
        };
        assert_eq!(validate_profile_for_kernel(&made_on_142, 142), []);
        assert_eq!(
            validate_profile_for_kernel(&made_on_142, 144),
            [
                warning(FpGpuVendor, RemovedInKernel, MajorRange::between(139, 143)),
                warning(
                    FpGpuRenderer,
                    RemovedInKernel,
                    MajorRange::between(139, 143)
                ),
                warning(
                    DisableGpuFingerprint,
                    RemovedInKernel,
                    MajorRange::between(142, 143)
                ),
            ]
        );

        let made_on_144 = ProfileDraft {
            fingerprint_seed: Some(1),
            disable_spoofing: vec![SpoofingFeature::Gpu],
            ..ProfileDraft::default()
        };
        assert_eq!(validate_profile_for_kernel(&made_on_144, 144), []);
        assert_eq!(
            validate_profile_for_kernel(&made_on_144, 142),
            [warning(
                DisableSpoofing,
                RequiresNewerKernel,
                MajorRange::since(144)
            )]
        );

        // Warnings are about the kernel version only; whether a seed is set does not matter.
        let no_seed = ProfileDraft {
            fingerprint_seed: None,
            ..made_on_142.clone()
        };
        assert_eq!(validate_profile_for_kernel(&no_seed, 144).len(), 3);
        // Validation never touches the draft, and building simply leaves the fields out.
        assert_eq!(
            argv(&made_on_142, 144, None),
            [
                "--user-data-dir=/data/profiles/p1",
                "--fingerprint=1",
                "--disable-non-proxied-udp"
            ]
        );
    }

    #[test]
    fn warnings_for_older_kernels() {
        use GatedField::*;
        use WarningReason::*;

        let draft = full_draft();
        assert_eq!(
            validate_profile_for_kernel(&draft, 130),
            [
                warning(FpBrand, RequiresNewerKernel, MajorRange::since(131)),
                warning(FpBrandVersion, RequiresNewerKernel, MajorRange::since(131)),
                warning(
                    FpGpuVendor,
                    RequiresNewerKernel,
                    MajorRange::between(139, 143)
                ),
                warning(
                    FpGpuRenderer,
                    RequiresNewerKernel,
                    MajorRange::between(139, 143)
                ),
                warning(
                    DisableGpuFingerprint,
                    RequiresNewerKernel,
                    MajorRange::between(142, 143)
                ),
                warning(DisableSpoofing, RequiresNewerKernel, MajorRange::since(144)),
            ]
        );
        assert_eq!(
            validate_profile_for_kernel(&draft, 139)
                .iter()
                .map(|w| w.field)
                .collect::<Vec<_>>(),
            [DisableGpuFingerprint, DisableSpoofing]
        );

        // Unset and blank fields never warn.
        assert_eq!(
            validate_profile_for_kernel(&ProfileDraft::default(), 130),
            []
        );
        let blank = ProfileDraft {
            fp_brand: Some(" ".into()),
            fp_gpu_vendor: Some(String::new()),
            ..ProfileDraft::default()
        };
        assert_eq!(validate_profile_for_kernel(&blank, 130), []);
        assert_eq!(validate_profile_for_kernel(&blank, 144), []);
    }

    /// Building and validating read the same version table: a set field warns exactly when
    /// the builder leaves its argument out.
    #[test]
    fn warnings_agree_with_arg_support() {
        let draft = full_draft();
        for major in 100..=200 {
            let support = ArgSupport::for_major(major);
            let warned: Vec<GatedField> = validate_profile_for_kernel(&draft, major)
                .iter()
                .map(|w| w.field)
                .collect();
            let unsupported: Vec<GatedField> = [
                (GatedField::FpBrand, support.brand),
                (GatedField::FpBrandVersion, support.brand),
                (GatedField::FpGpuVendor, support.gpu_vendor_renderer),
                (GatedField::FpGpuRenderer, support.gpu_vendor_renderer),
                (
                    GatedField::DisableGpuFingerprint,
                    support.disable_gpu_fingerprint,
                ),
                (GatedField::DisableSpoofing, support.disable_spoofing),
            ]
            .into_iter()
            .filter(|&(_, supported)| !supported)
            .map(|(field, _)| field)
            .collect();
            assert_eq!(warned, unsupported, "major {major}");
        }
    }
}

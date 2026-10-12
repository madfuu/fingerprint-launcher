//! Profile export and import (R1.5, design §5): a JSON file for backups and moving profiles to
//! another machine.
//!
//! The file holds only what is portable. Ids, user data directories and the kernel binding are
//! machine-specific and left out; groups and tags travel by name, proxies by
//! scheme/host/port. Importing reuses a group, tag or proxy that matches and creates the
//! missing ones; every imported profile is new.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::groups::{self, GroupInput};
use crate::db::profiles::{self, FpPlatform, ProfileDraft, SpoofingFeature};
use crate::db::proxies::{self, ProxyInput, ProxyScheme};
use crate::db::tags::{self, TagInput};
use crate::db::{in_tx, new_id, now, require_absolute};
use crate::error::{AppError, AppResult};
use crate::services::profiles::create_checked;
use crate::state::AppState;

/// Version of the file format. Files of a newer version are refused.
pub const FORMAT_VERSION: u32 = 1;
/// Larger files are refused before they are parsed.
const MAX_IMPORT_BYTES: u64 = 32 * 1024 * 1024;
/// More profiles in one file are refused while parsing. Each parsed profile takes far more
/// memory than its shortest JSON form (`{}`), so the byte limit alone would still let a hostile
/// file allocate gigabytes.
const MAX_IMPORT_PROFILES: usize = 10_000;

/// Counts reported by `importProfiles`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: u32,
    pub groups_created: u32,
    pub tags_created: u32,
    pub proxies_created: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile {
    format_version: u32,
    exported_at: String,
    #[serde(deserialize_with = "bounded_profiles")]
    profiles: Vec<PortableProfile>,
}

/// `profiles`, refusing more than [`MAX_IMPORT_PROFILES`] before building the next one.
fn bounded_profiles<'de, D>(deserializer: D) -> Result<Vec<PortableProfile>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Bounded;

    impl<'de> serde::de::Visitor<'de> for Bounded {
        type Value = Vec<PortableProfile>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a list of at most {MAX_IMPORT_PROFILES} profiles")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut profiles = Vec::new();
            while profiles.len() < MAX_IMPORT_PROFILES {
                match seq.next_element()? {
                    Some(profile) => profiles.push(profile),
                    None => return Ok(profiles),
                }
            }
            match seq.next_element::<serde::de::IgnoredAny>()? {
                None => Ok(profiles),
                Some(_) => Err(serde::de::Error::custom(format_args!(
                    "more than {MAX_IMPORT_PROFILES} profiles in one file"
                ))),
            }
        }
    }

    deserializer.deserialize_seq(Bounded)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableGroup {
    name: String,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableTag {
    name: String,
    color: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableProxy {
    scheme: ProxyScheme,
    host: String,
    port: u16,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

/// A profile in the file: [`ProfileDraft`] without the machine-specific fields, with its
/// group, tags and proxy by value. Missing fields take the draft defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PortableProfile {
    name: String,
    note: Option<String>,
    group: Option<PortableGroup>,
    tags: Vec<PortableTag>,
    proxy: Option<PortableProxy>,
    fingerprint_seed: Option<u32>,
    fp_platform: Option<FpPlatform>,
    fp_platform_version: Option<String>,
    fp_brand: Option<String>,
    fp_brand_version: Option<String>,
    fp_hardware_concurrency: Option<u32>,
    fp_gpu_vendor: Option<String>,
    fp_gpu_renderer: Option<String>,
    disable_gpu_fingerprint: bool,
    disable_spoofing: Vec<SpoofingFeature>,
    disable_non_proxied_udp: bool,
    lang: Option<String>,
    accept_lang: Option<String>,
    timezone: Option<String>,
    headless: bool,
    start_urls: Vec<String>,
    extra_args: Option<String>,
}

impl Default for PortableProfile {
    fn default() -> Self {
        Self::new(ProfileDraft::default(), None, Vec::new(), None)
    }
}

impl PortableProfile {
    fn new(
        draft: ProfileDraft,
        group: Option<PortableGroup>,
        tags: Vec<PortableTag>,
        proxy: Option<PortableProxy>,
    ) -> Self {
        // Destructured so that a new draft field fails to compile here until it is decided
        // whether it travels.
        let ProfileDraft {
            name,
            note,
            group_id: _,
            kernel_id: _,
            proxy_id: _,
            tag_ids: _,
            fingerprint_seed,
            fp_platform,
            fp_platform_version,
            fp_brand,
            fp_brand_version,
            fp_hardware_concurrency,
            fp_gpu_vendor,
            fp_gpu_renderer,
            disable_gpu_fingerprint,
            disable_spoofing,
            disable_non_proxied_udp,
            lang,
            accept_lang,
            timezone,
            headless,
            user_data_dir: _,
            start_urls,
            extra_args,
        } = draft;
        Self {
            name,
            note,
            group,
            tags,
            proxy,
            fingerprint_seed,
            fp_platform,
            fp_platform_version,
            fp_brand,
            fp_brand_version,
            fp_hardware_concurrency,
            fp_gpu_vendor,
            fp_gpu_renderer,
            disable_gpu_fingerprint,
            disable_spoofing,
            disable_non_proxied_udp,
            lang,
            accept_lang,
            timezone,
            headless,
            start_urls,
            extra_args,
        }
    }

    fn into_draft(
        self,
        group_id: Option<String>,
        tag_ids: Vec<String>,
        proxy_id: Option<String>,
    ) -> ProfileDraft {
        ProfileDraft {
            name: self.name,
            note: self.note,
            group_id,
            kernel_id: None,
            proxy_id,
            tag_ids,
            fingerprint_seed: self.fingerprint_seed,
            fp_platform: self.fp_platform,
            fp_platform_version: self.fp_platform_version,
            fp_brand: self.fp_brand,
            fp_brand_version: self.fp_brand_version,
            fp_hardware_concurrency: self.fp_hardware_concurrency,
            fp_gpu_vendor: self.fp_gpu_vendor,
            fp_gpu_renderer: self.fp_gpu_renderer,
            disable_gpu_fingerprint: self.disable_gpu_fingerprint,
            disable_spoofing: self.disable_spoofing,
            disable_non_proxied_udp: self.disable_non_proxied_udp,
            lang: self.lang,
            accept_lang: self.accept_lang,
            timezone: self.timezone,
            headless: self.headless,
            user_data_dir: None,
            start_urls: self.start_urls,
            extra_args: self.extra_args,
        }
    }
}

/// Writes the profiles `ids` (in that order) to `path`, replacing the file; returns how many.
pub async fn export_profiles(state: &AppState, ids: Vec<String>, path: String) -> AppResult<u32> {
    let path = PathBuf::from(target_path(&path)?);
    if ids.is_empty() {
        return Err(AppError::invalid_input("no profiles to export"));
    }
    let file = state.db(move |conn| build_export(conn, &ids)).await?;
    let count = u32::try_from(file.profiles.len()).unwrap_or(u32::MAX);
    let json = serde_json::to_vec_pretty(&file)
        .map_err(|err| AppError::invalid_input(format!("cannot encode export: {err}")))?;
    tokio::task::spawn_blocking(move || write_replacing(&path, &json))
        .await
        .map_err(|err| AppError::new(crate::error::ErrorCode::Io, err.to_string()))??;
    Ok(count)
}

/// Imports every profile of the file at `path`, all or nothing.
pub async fn import_profiles(state: &AppState, path: String) -> AppResult<ImportResult> {
    let path = PathBuf::from(target_path(&path)?);
    let default_root = state.paths.default_user_data_root();
    let file = tokio::task::spawn_blocking(move || read_export(&path))
        .await
        .map_err(|err| AppError::new(crate::error::ErrorCode::Io, err.to_string()))??;
    state
        .db(move |conn| in_tx(conn, |conn| apply_import(conn, &default_root, file)))
        .await
}

fn target_path(path: &str) -> AppResult<&str> {
    let path = path.trim();
    require_absolute(path, "file path")?;
    Ok(path)
}

fn build_export(conn: &Connection, ids: &[String]) -> AppResult<ExportFile> {
    let mut portable = Vec::with_capacity(ids.len());
    for id in ids {
        let profile = profiles::get(conn, id)?;
        let draft = profile.draft;
        let group = draft
            .group_id
            .as_deref()
            .map(|id| groups::get(conn, id))
            .transpose()?
            .map(|g| PortableGroup {
                name: g.name,
                color: g.color,
            });
        let tags = draft
            .tag_ids
            .iter()
            .map(|id| {
                tags::get(conn, id).map(|t| PortableTag {
                    name: t.name,
                    color: t.color,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        let proxy = draft
            .proxy_id
            .as_deref()
            .map(|id| proxies::get(conn, id))
            .transpose()?
            .map(|p| PortableProxy {
                scheme: p.scheme,
                host: p.host,
                port: p.port,
                name: p.name,
                note: p.note,
            });
        portable.push(PortableProfile::new(draft, group, tags, proxy));
    }
    Ok(ExportFile {
        format_version: FORMAT_VERSION,
        exported_at: now(),
        profiles: portable,
    })
}

/// Writes through a temporary file in the same directory and renames it over `path`, so a
/// failed write never leaves a truncated export behind.
fn write_replacing(path: &Path, data: &[u8]) -> AppResult<()> {
    let name = path
        .file_name()
        .ok_or_else(|| AppError::invalid_input(format!("not a file path: {}", path.display())))?;
    let mut temp_name = name.to_owned();
    temp_name.push(format!(".{}.tmp", new_id()));
    let temp = path.with_file_name(temp_name);
    let result = std::fs::write(&temp, data).and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    Ok(result?)
}

fn read_export(path: &Path) -> AppResult<ExportFile> {
    use std::io::Read;

    // Reads one byte past the limit instead of trusting the size reported beforehand.
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_IMPORT_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_IMPORT_BYTES {
        return Err(AppError::invalid_input(format!(
            "file is too large for a profile export (over {MAX_IMPORT_BYTES} bytes)"
        )));
    }
    parse_export(&data)
}

fn parse_export(data: &[u8]) -> AppResult<ExportFile> {
    /// Only the version; every other field is skipped without being built.
    #[derive(Deserialize)]
    struct Header {
        #[serde(rename = "formatVersion")]
        format_version: Option<u64>,
    }

    let not_export = |err: serde_json::Error| {
        AppError::invalid_input(format!("not a profile export file: {err}"))
    };
    // Checked first, so a newer file says so instead of failing on a field it added.
    let header: Header = serde_json::from_slice(data).map_err(not_export)?;
    match header.format_version {
        Some(version) if (1..=u64::from(FORMAT_VERSION)).contains(&version) => {}
        Some(version) => {
            return Err(AppError::invalid_input(format!(
                "export format version {version} is not supported (this build reads up to \
                 {FORMAT_VERSION})"
            )));
        }
        None => {
            return Err(AppError::invalid_input(
                "not a profile export file: formatVersion is missing",
            ));
        }
    }
    serde_json::from_slice(data).map_err(not_export)
}

fn apply_import(
    conn: &Connection,
    default_root: &Path,
    file: ExportFile,
) -> AppResult<ImportResult> {
    let mut result = ImportResult::default();
    for (index, profile) in file.profiles.into_iter().enumerate() {
        let name = profile.name.clone();
        let context = |err: AppError| {
            AppError::new(
                err.code,
                format!("profile {} \"{name}\": {}", index + 1, err.message),
            )
        };
        let group_id = profile
            .group
            .clone()
            .map(|group| import_group(conn, group, &mut result))
            .transpose()
            .map_err(context)?;
        let tag_ids = profile
            .tags
            .clone()
            .into_iter()
            .map(|tag| import_tag(conn, tag, &mut result))
            .collect::<AppResult<Vec<_>>>()
            .map_err(context)?;
        let proxy_id = profile
            .proxy
            .clone()
            .map(|proxy| import_proxy(conn, proxy, &mut result))
            .transpose()
            .map_err(context)?;
        create_checked(
            conn,
            default_root,
            profile.into_draft(group_id, tag_ids, proxy_id),
        )
        .map_err(context)?;
        result.imported += 1;
    }
    Ok(result)
}

fn import_group(
    conn: &Connection,
    group: PortableGroup,
    result: &mut ImportResult,
) -> AppResult<String> {
    if let Some(existing) = groups::find_by_name(conn, &group.name)? {
        return Ok(existing.id);
    }
    let created = groups::create(
        conn,
        GroupInput {
            name: group.name,
            color: group.color,
            sort_order: 0,
        },
    )?;
    result.groups_created += 1;
    Ok(created.id)
}

fn import_tag(conn: &Connection, tag: PortableTag, result: &mut ImportResult) -> AppResult<String> {
    if let Some(existing) = tags::find_by_name(conn, &tag.name)? {
        return Ok(existing.id);
    }
    let created = tags::create(
        conn,
        TagInput {
            name: tag.name,
            color: tag.color,
        },
    )?;
    result.tags_created += 1;
    Ok(created.id)
}

fn import_proxy(
    conn: &Connection,
    proxy: PortableProxy,
    result: &mut ImportResult,
) -> AppResult<String> {
    if let Some(existing) = proxies::find_by_endpoint(conn, proxy.scheme, &proxy.host, proxy.port)?
    {
        return Ok(existing.id);
    }
    let created = proxies::create(
        conn,
        ProxyInput {
            name: proxy.name,
            scheme: proxy.scheme,
            host: proxy.host,
            port: proxy.port,
            note: proxy.note,
        },
    )?;
    result.proxies_created += 1;
    Ok(created.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profiles::{Profile, ProfileQuery, ProfileView};
    use crate::error::ErrorCode;
    use crate::state::test_support::TestState;

    async fn seed(state: &AppState) -> Vec<Profile> {
        state
            .db(|conn| {
                let work = groups::create(
                    conn,
                    GroupInput {
                        name: "Work".into(),
                        color: Some("#3b82f6".into()),
                        sort_order: 0,
                    },
                )?;
                let vip = tags::create(
                    conn,
                    TagInput {
                        name: "vip".into(),
                        color: "#f00".into(),
                    },
                )?;
                let ads = tags::create(
                    conn,
                    TagInput {
                        name: "ads".into(),
                        color: "#0f0".into(),
                    },
                )?;
                let proxy = proxies::create(
                    conn,
                    ProxyInput {
                        name: Some("Home".into()),
                        scheme: ProxyScheme::Socks5,
                        host: "::1".into(),
                        port: 1080,
                        note: None,
                    },
                )?;
                let kernel = crate::db::kernels::create(
                    conn,
                    crate::db::kernels::KernelInput {
                        name: "k".into(),
                        exec_path: std::env::temp_dir()
                            .join("chrome")
                            .to_str()
                            .unwrap()
                            .to_owned(),
                        major_version: 144,
                        full_version: None,
                    },
                )?;
                let full = profiles::create(
                    conn,
                    ProfileDraft {
                        name: "Shop".into(),
                        note: Some("main".into()),
                        group_id: Some(work.id),
                        kernel_id: Some(kernel.id),
                        proxy_id: Some(proxy.id),
                        tag_ids: vec![vip.id, ads.id],
                        fingerprint_seed: Some(2023),
                        fp_platform: Some(FpPlatform::Macos),
                        fp_platform_version: Some("15.2.0".into()),
                        fp_brand: Some("Edge".into()),
                        disable_spoofing: vec![SpoofingFeature::Font],
                        disable_non_proxied_udp: false,
                        lang: Some("zh-CN".into()),
                        timezone: Some("Asia/Shanghai".into()),
                        headless: true,
                        user_data_dir: Some(
                            std::env::temp_dir()
                                .join("custom-udd")
                                .to_str()
                                .unwrap()
                                .to_owned(),
                        ),
                        start_urls: vec!["https://example.com".into()],
                        extra_args: Some("--window-name='My Shop'".into()),
                        ..ProfileDraft::default()
                    },
                )?;
                let bare = profiles::create(
                    conn,
                    ProfileDraft {
                        name: "Bare".into(),
                        ..ProfileDraft::default()
                    },
                )?;
                Ok(vec![full, bare])
            })
            .await
            .unwrap()
    }

    fn export_path(t: &TestState) -> String {
        t.dir
            .path()
            .join("export.json")
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn round_trip_into_another_installation() {
        let source = TestState::new();
        let originals = seed(&source.state).await;
        let path = export_path(&source);
        let ids = originals.iter().map(|p| p.id.clone()).collect();
        assert_eq!(
            export_profiles(&source.state, ids, path.clone())
                .await
                .unwrap(),
            2
        );

        let text = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["formatVersion"], 1);
        let first = &json["profiles"][0];
        for machine_specific in [
            "id",
            "userDataDir",
            "kernelId",
            "groupId",
            "proxyId",
            "tagIds",
        ] {
            assert!(first.get(machine_specific).is_none(), "{machine_specific}");
        }
        assert_eq!(first["group"]["name"], "Work");
        assert_eq!(first["proxy"]["host"], "::1");
        assert_eq!(first["tags"][0]["name"], "ads");

        // A second installation that already has the group (in another case) and the proxy.
        let target = TestState::new();
        let existing_proxy = target
            .state
            .db(|conn| {
                groups::create(
                    conn,
                    GroupInput {
                        name: "WORK".into(),
                        color: None,
                        sort_order: 0,
                    },
                )?;
                proxies::create(
                    conn,
                    ProxyInput {
                        name: None,
                        scheme: ProxyScheme::Socks5,
                        host: "::1".into(),
                        port: 1080,
                        note: None,
                    },
                )
            })
            .await
            .unwrap();
        let result = import_profiles(&target.state, path.clone()).await.unwrap();
        assert_eq!(
            result,
            ImportResult {
                imported: 2,
                groups_created: 0,
                tags_created: 2,
                proxies_created: 0,
            }
        );

        let imported = target
            .state
            .db(|conn| {
                let page = profiles::list(
                    conn,
                    &ProfileQuery {
                        view: ProfileView::All,
                        ..ProfileQuery::default()
                    },
                    &[],
                )?;
                Ok(page.items)
            })
            .await
            .unwrap();
        let shop = imported
            .iter()
            .find(|row| row.profile.draft.name == "Shop")
            .unwrap();
        let original = &originals[0].draft;
        let copy = &shop.profile.draft;
        assert_ne!(shop.profile.id, originals[0].id);
        assert_eq!(shop.group.as_ref().unwrap().name, "WORK");
        assert_eq!(copy.proxy_id.as_ref(), Some(&existing_proxy.id));
        assert_eq!(
            shop.tags
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            ["ads", "vip"]
        );
        // Machine-specific fields are not carried over; everything else is.
        assert_eq!(copy.kernel_id, None);
        assert_eq!(copy.user_data_dir, None);
        let comparable = |d: &ProfileDraft| ProfileDraft {
            group_id: None,
            kernel_id: None,
            proxy_id: None,
            tag_ids: Vec::new(),
            user_data_dir: None,
            ..d.clone()
        };
        assert_eq!(comparable(copy), comparable(original));

        // Importing again creates new profiles and reuses everything else.
        let again = import_profiles(&target.state, path).await.unwrap();
        assert_eq!(
            again,
            ImportResult {
                imported: 2,
                ..ImportResult::default()
            }
        );
    }

    #[tokio::test]
    async fn import_is_all_or_nothing() {
        let t = TestState::new();
        let path = export_path(&t);
        std::fs::write(
            &path,
            r##"{ "formatVersion": 1, "exportedAt": "x", "profiles": [
                { "name": "ok", "tags": [{ "name": "new", "color": "#fff" }] },
                { "name": "bad", "fingerprintSeed": 0 }
            ] }"##,
        )
        .unwrap();
        let err = import_profiles(&t.state, path).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(
            err.message.starts_with("profile 2 \"bad\": "),
            "{}",
            err.message
        );
        let counts = t
            .state
            .db(|conn| {
                let count = |table: &str| -> AppResult<i64> {
                    Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
                };
                Ok((count("profiles")?, count("tags")?))
            })
            .await
            .unwrap();
        assert_eq!(counts, (0, 0));
    }

    #[test]
    fn rejects_files_that_are_not_exports_or_too_new() {
        let code = |text: &str| parse_export(text.as_bytes()).unwrap_err();
        for bad in [
            "",
            "[]",
            "{}",
            r#"{ "formatVersion": 1 }"#,
            r#"{ "formatVersion": 0 }"#,
        ] {
            assert_eq!(code(bad).code, ErrorCode::InvalidInput, "{bad}");
        }
        let err = code(r#"{ "formatVersion": 2, "profiles": "new shape" }"#);
        assert!(err.message.contains("version 2"), "{}", err.message);
        // Missing profile fields take the draft defaults.
        let file = parse_export(
            br#"{ "formatVersion": 1, "exportedAt": "x", "profiles": [{ "name": "p" }] }"#,
        )
        .unwrap();
        let draft = file.profiles[0].clone().into_draft(None, Vec::new(), None);
        assert_eq!(
            draft,
            ProfileDraft {
                name: "p".into(),
                ..ProfileDraft::default()
            }
        );
    }

    #[test]
    fn refuses_more_profiles_than_the_limit_while_parsing() {
        let file = |count: usize| {
            let profiles = vec![r#"{"name":"p"}"#; count].join(",");
            format!(r#"{{ "formatVersion": 1, "exportedAt": "x", "profiles": [{profiles}] }}"#)
        };
        let parsed = parse_export(file(MAX_IMPORT_PROFILES).as_bytes()).unwrap();
        assert_eq!(parsed.profiles.len(), MAX_IMPORT_PROFILES);
        let err = parse_export(file(MAX_IMPORT_PROFILES + 1).as_bytes()).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(err.message.contains("more than"), "{}", err.message);
    }

    #[tokio::test]
    async fn refuses_files_over_the_size_limit() {
        let t = TestState::new();
        let path = export_path(&t);
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_IMPORT_BYTES + 1).unwrap();
        drop(file);
        let err = import_profiles(&t.state, path).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(err.message.contains("too large"), "{}", err.message);
    }

    #[tokio::test]
    async fn export_errors() {
        let t = TestState::new();
        let profiles = seed(&t.state).await;
        let ids = vec![profiles[0].id.clone()];
        let code = |r: AppResult<u32>| r.unwrap_err().code;
        assert_eq!(
            code(export_profiles(&t.state, ids.clone(), "relative.json".into()).await),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(export_profiles(&t.state, Vec::new(), export_path(&t)).await),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(export_profiles(&t.state, vec!["missing".into()], export_path(&t)).await),
            ErrorCode::NotFound
        );
        let in_missing_dir = t.dir.path().join("no-such-dir").join("x.json");
        assert_eq!(
            code(export_profiles(&t.state, ids, in_missing_dir.to_str().unwrap().to_owned()).await),
            ErrorCode::Io
        );
        // Nothing half-written next to the target.
        assert!(!t.dir.path().join("no-such-dir").exists());
    }
}

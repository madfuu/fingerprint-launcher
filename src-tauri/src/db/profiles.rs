//! Profile repository (design §2.2 `profiles` + `profile_tags`).
//!
//! Only configuration is stored. Whether a profile is running lives in the launcher's
//! in-memory table (design decision 2), so [`list`] takes the running ids from its caller.

use std::collections::HashMap;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, Value, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::groups::{self, Group};
use super::proxies::{self, Proxy};
use super::tags::{self, Tag};
use super::{
    Page, clean_opt, col, ensure_found, in_tx, json_string_array, new_id, not_found, now,
    require_absolute, required_text, select_list,
};
use crate::error::{AppError, AppResult};
use crate::kernel::args::{check_start_urls, split_extra_args};

text_enum! {
    /// `--fingerprint-platform` value.
    pub enum FpPlatform {
        Windows => "windows",
        Linux => "linux",
        Macos => "macos",
    }
}

text_enum! {
    /// A feature accepted by `--disable-spoofing` (Chrome 144+). Variant order is the canonical
    /// order of the stored CSV.
    pub enum SpoofingFeature {
        Font => "font",
        Audio => "audio",
        Canvas => "canvas",
        ClientRects => "clientrects",
        Gpu => "gpu",
    }
}

/// Largest `--fingerprint` seed. Seeds are kept to positive signed 32-bit values so every parser
/// reads them the same way (research/fingerprint-chromium-cli.md §5).
pub const SEED_MAX: u32 = 2_147_483_647;

pub const DEFAULT_PAGE_SIZE: u32 = 20;
pub const MAX_PAGE_SIZE: u32 = 500;

const COLUMNS: &[&str] = &[
    "id",
    "name",
    "note",
    "group_id",
    "kernel_id",
    "proxy_id",
    "fingerprint_seed",
    "fp_platform",
    "fp_platform_version",
    "fp_brand",
    "fp_brand_version",
    "fp_hardware_concurrency",
    "fp_gpu_vendor",
    "fp_gpu_renderer",
    "disable_gpu_fingerprint",
    "disable_spoofing",
    "disable_non_proxied_udp",
    "lang",
    "accept_lang",
    "timezone",
    "headless",
    "user_data_dir",
    "start_urls",
    "extra_args",
    "last_launched_at",
    "deleted_at",
    "created_at",
    "updated_at",
];

/// The editable part of a profile: what the editor saves, what `previewCommand` renders and
/// what the argument builder consumes. This is the bridge's `ProfileDraft`.
///
/// `None` / empty means "do not pass the argument". Missing JSON fields take the defaults of
/// [`ProfileDraft::default`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileDraft {
    pub name: String,
    pub note: Option<String>,
    pub group_id: Option<String>,
    /// `None` = the default kernel.
    pub kernel_id: Option<String>,
    pub proxy_id: Option<String>,
    /// Stored in `profile_tags`; read back ordered by tag name.
    pub tag_ids: Vec<String>,

    /// `None` = no `--fingerprint`, so the browser behaves like stock Chromium.
    pub fingerprint_seed: Option<u32>,
    pub fp_platform: Option<FpPlatform>,
    pub fp_platform_version: Option<String>,
    pub fp_brand: Option<String>,
    pub fp_brand_version: Option<String>,
    pub fp_hardware_concurrency: Option<u32>,

    /// Chrome 139–143.
    pub fp_gpu_vendor: Option<String>,
    /// Chrome 139–143.
    pub fp_gpu_renderer: Option<String>,
    /// Chrome 142–143.
    pub disable_gpu_fingerprint: bool,
    /// Chrome 144+. Stored as CSV in canonical order.
    pub disable_spoofing: Vec<SpoofingFeature>,

    pub disable_non_proxied_udp: bool,
    pub lang: Option<String>,
    pub accept_lang: Option<String>,
    pub timezone: Option<String>,
    pub headless: bool,

    /// `None` = `<user_data_root>/<profile id>`.
    pub user_data_dir: Option<String>,
    /// Stored as a JSON array.
    pub start_urls: Vec<String>,
    /// Stored verbatim; split into words by `kernel::args::split_extra_args` and appended after
    /// the generated arguments.
    pub extra_args: Option<String>,
}

impl Default for ProfileDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            note: None,
            group_id: None,
            kernel_id: None,
            proxy_id: None,
            tag_ids: Vec::new(),
            fingerprint_seed: None,
            fp_platform: None,
            fp_platform_version: None,
            fp_brand: None,
            fp_brand_version: None,
            fp_hardware_concurrency: None,
            fp_gpu_vendor: None,
            fp_gpu_renderer: None,
            disable_gpu_fingerprint: false,
            disable_spoofing: Vec::new(),
            // WebRTC leak protection is on unless the user turns it off (R2.1).
            disable_non_proxied_udp: true,
            lang: None,
            accept_lang: None,
            timezone: None,
            headless: false,
            user_data_dir: None,
            start_urls: Vec::new(),
            extra_args: None,
        }
    }
}

impl ProfileDraft {
    /// Trims text, turns blank optional text into NULL, canonicalises the list fields and
    /// rejects values that are invalid for any kernel. Version-dependent checks belong to the
    /// argument builder (design §3.2).
    fn normalized(mut self) -> AppResult<Self> {
        self.name = required_text(&self.name, "profile name")?;
        for field in [
            &mut self.note,
            &mut self.group_id,
            &mut self.kernel_id,
            &mut self.proxy_id,
            &mut self.fp_platform_version,
            &mut self.fp_brand,
            &mut self.fp_brand_version,
            &mut self.fp_gpu_vendor,
            &mut self.fp_gpu_renderer,
            &mut self.lang,
            &mut self.accept_lang,
            &mut self.timezone,
            &mut self.user_data_dir,
            &mut self.extra_args,
        ] {
            clean_opt(field);
        }
        if self
            .fingerprint_seed
            .is_some_and(|seed| !(1..=SEED_MAX).contains(&seed))
        {
            return Err(AppError::invalid_input(format!(
                "fingerprint seed must be 1-{SEED_MAX}"
            )));
        }
        if self.fp_hardware_concurrency == Some(0) {
            return Err(AppError::invalid_input(
                "hardware concurrency must be positive",
            ));
        }
        if let Some(dir) = &self.user_data_dir {
            require_absolute(dir, "user data directory")?;
        }
        if let Some(extra) = &self.extra_args {
            // Unbalanced quotes fail here, on save, rather than on every launch.
            split_extra_args(extra)?;
        }
        self.tag_ids = normalize_ids(&self.tag_ids);
        self.disable_spoofing.sort_unstable();
        self.disable_spoofing.dedup();
        self.start_urls = self
            .start_urls
            .iter()
            .map(|url| url.trim())
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
            .collect();
        check_start_urls(&self.start_urls)?;
        Ok(self)
    }
}

/// A stored profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    #[serde(flatten)]
    pub draft: ProfileDraft,
    pub last_launched_at: Option<String>,
    /// Set while the profile is in the trash.
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A list row: the profile plus the group, proxy and tags it references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRow {
    #[serde(flatten)]
    pub profile: Profile,
    pub group: Option<Group>,
    pub proxy: Option<Proxy>,
    /// Ordered by name.
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileView {
    /// Every profile not in the trash.
    #[default]
    All,
    /// Profiles not in the trash whose id is in the caller's running set.
    Running,
    /// Profiles in the trash, most recently deleted first.
    Trash,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileQuery {
    /// 1-based.
    pub page: u32,
    /// 1..=[`MAX_PAGE_SIZE`].
    pub page_size: u32,
    pub view: ProfileView,
    pub group_id: Option<String>,
    /// Matches profiles carrying at least one of these tags.
    pub tag_ids: Vec<String>,
    /// Substring of the name, note or seed; case-insensitive for ASCII.
    pub keyword: Option<String>,
}

impl Default for ProfileQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: DEFAULT_PAGE_SIZE,
            view: ProfileView::All,
            group_id: None,
            tag_ids: Vec::new(),
            keyword: None,
        }
    }
}

/// `disable_spoofing` column: CSV of [`SpoofingFeature`] literals.
struct SpoofingCsv(Vec<SpoofingFeature>);

impl FromSql for SpoofingCsv {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map(Self)
            .map_err(FromSqlError::other)
    }
}

/// `start_urls` column: JSON array of strings.
struct StringList(Vec<String>);

impl FromSql for StringList {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        serde_json::from_str(value.as_str()?)
            .map(Self)
            .map_err(FromSqlError::other)
    }
}

/// `font,clientrects,...`: canonical order, no duplicates, `None` when empty. The form of both
/// the `disable_spoofing` column and the `--disable-spoofing` value.
pub(crate) fn spoofing_csv(features: &[SpoofingFeature]) -> Option<String> {
    let mut features = features.to_vec();
    features.sort_unstable();
    features.dedup();
    (!features.is_empty()).then(|| {
        features
            .iter()
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
            .join(",")
    })
}

fn string_list_json(values: &[String]) -> Option<String> {
    (!values.is_empty()).then(|| json_string_array(values))
}

/// Trimmed, non-empty, sorted and deduplicated.
fn normalize_ids(ids: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Reads a profile from columns selected with `select_list(COLUMNS, alias, "")`. `tag_ids` is
/// left empty; callers fill it from `profile_tags`.
fn from_row(row: &Row<'_>) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: row.get("id")?,
        draft: ProfileDraft {
            name: row.get("name")?,
            note: row.get("note")?,
            group_id: row.get("group_id")?,
            kernel_id: row.get("kernel_id")?,
            proxy_id: row.get("proxy_id")?,
            tag_ids: Vec::new(),
            fingerprint_seed: row.get("fingerprint_seed")?,
            fp_platform: row.get("fp_platform")?,
            fp_platform_version: row.get("fp_platform_version")?,
            fp_brand: row.get("fp_brand")?,
            fp_brand_version: row.get("fp_brand_version")?,
            fp_hardware_concurrency: row.get("fp_hardware_concurrency")?,
            fp_gpu_vendor: row.get("fp_gpu_vendor")?,
            fp_gpu_renderer: row.get("fp_gpu_renderer")?,
            disable_gpu_fingerprint: row.get("disable_gpu_fingerprint")?,
            disable_spoofing: row
                .get::<_, Option<SpoofingCsv>>("disable_spoofing")?
                .map_or_else(Vec::new, |csv| csv.0),
            disable_non_proxied_udp: row.get("disable_non_proxied_udp")?,
            lang: row.get("lang")?,
            accept_lang: row.get("accept_lang")?,
            timezone: row.get("timezone")?,
            headless: row.get("headless")?,
            user_data_dir: row.get("user_data_dir")?,
            start_urls: row
                .get::<_, Option<StringList>>("start_urls")?
                .map_or_else(Vec::new, |list| list.0),
            extra_args: row.get("extra_args")?,
        },
        last_launched_at: row.get("last_launched_at")?,
        deleted_at: row.get("deleted_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// Column / value pairs of every editable column. INSERT and UPDATE are both generated from
/// this list, so they cannot drift apart.
fn editable_columns(d: &ProfileDraft) -> Vec<(&'static str, Box<dyn ToSql + '_>)> {
    fn v<'a>(value: impl ToSql + 'a) -> Box<dyn ToSql + 'a> {
        Box::new(value)
    }
    vec![
        ("name", v(&d.name)),
        ("note", v(&d.note)),
        ("group_id", v(&d.group_id)),
        ("kernel_id", v(&d.kernel_id)),
        ("proxy_id", v(&d.proxy_id)),
        ("fingerprint_seed", v(d.fingerprint_seed)),
        ("fp_platform", v(d.fp_platform)),
        ("fp_platform_version", v(&d.fp_platform_version)),
        ("fp_brand", v(&d.fp_brand)),
        ("fp_brand_version", v(&d.fp_brand_version)),
        ("fp_hardware_concurrency", v(d.fp_hardware_concurrency)),
        ("fp_gpu_vendor", v(&d.fp_gpu_vendor)),
        ("fp_gpu_renderer", v(&d.fp_gpu_renderer)),
        ("disable_gpu_fingerprint", v(d.disable_gpu_fingerprint)),
        ("disable_spoofing", v(spoofing_csv(&d.disable_spoofing))),
        ("disable_non_proxied_udp", v(d.disable_non_proxied_udp)),
        ("lang", v(&d.lang)),
        ("accept_lang", v(&d.accept_lang)),
        ("timezone", v(&d.timezone)),
        ("headless", v(d.headless)),
        ("user_data_dir", v(&d.user_data_dir)),
        ("start_urls", v(string_list_json(&d.start_urls))),
        ("extra_args", v(&d.extra_args)),
    ]
}

fn select(where_clause: &str) -> String {
    format!(
        "SELECT {} FROM profiles p {where_clause}",
        select_list(COLUMNS, "p", "")
    )
}

pub fn create(conn: &Connection, draft: ProfileDraft) -> AppResult<Profile> {
    let draft = draft.normalized()?;
    let id = new_id();
    let now = now();
    let columns = editable_columns(&draft);
    let names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
    let sql = format!(
        "INSERT INTO profiles (id, created_at, updated_at, {}) VALUES (?, ?, ?{})",
        names.join(", "),
        ", ?".repeat(names.len())
    );
    let mut values: Vec<&dyn ToSql> = vec![&id, &now, &now];
    values.extend(columns.iter().map(|(_, value)| &**value as &dyn ToSql));

    in_tx(conn, |conn| {
        conn.execute(&sql, values.as_slice())?;
        replace_tags(conn, &id, &draft.tag_ids)
    })?;
    get(conn, &id)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Profile> {
    let mut profile = conn
        .query_row(&select("WHERE p.id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| not_found("profile", id))?;
    profile.draft.tag_ids = load_tags(conn, &[profile.id.clone()])?
        .remove(&profile.id)
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.id)
        .collect();
    Ok(profile)
}

/// Replaces every editable field, including the tag set. Works on profiles in the trash too.
pub fn update(conn: &Connection, id: &str, draft: ProfileDraft) -> AppResult<Profile> {
    let draft = draft.normalized()?;
    let now = now();
    let columns = editable_columns(&draft);
    let assignments = columns
        .iter()
        .map(|(name, _)| format!("{name} = ?"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("UPDATE profiles SET updated_at = ?, {assignments} WHERE id = ?");
    let mut values: Vec<&dyn ToSql> = vec![&now];
    values.extend(columns.iter().map(|(_, value)| &**value as &dyn ToSql));
    values.push(&id);

    in_tx(conn, |conn| {
        let changed = conn.execute(&sql, values.as_slice())?;
        ensure_found(changed, "profile", id)?;
        replace_tags(conn, id, &draft.tag_ids)
    })?;
    get(conn, id)
}

/// One page of profiles for the list view (design §5 `ProfileQuery`). `running_ids` is only
/// consulted for [`ProfileView::Running`].
pub fn list(
    conn: &Connection,
    query: &ProfileQuery,
    running_ids: &[String],
) -> AppResult<Page<ProfileRow>> {
    if query.page == 0 {
        return Err(AppError::invalid_input("page is 1-based"));
    }
    if !(1..=MAX_PAGE_SIZE).contains(&query.page_size) {
        return Err(AppError::invalid_input(format!(
            "page size must be 1-{MAX_PAGE_SIZE}"
        )));
    }

    let mut clauses: Vec<&str> = Vec::new();
    let mut params: Vec<(&str, Value)> = Vec::new();
    match query.view {
        ProfileView::All => clauses.push("p.deleted_at IS NULL"),
        ProfileView::Running => {
            clauses.push("p.deleted_at IS NULL");
            clauses.push("p.id IN (SELECT value FROM json_each(:running_ids))");
            params.push((":running_ids", json_string_array(running_ids).into()));
        }
        ProfileView::Trash => clauses.push("p.deleted_at IS NOT NULL"),
    }
    if let Some(group_id) = non_blank(query.group_id.as_deref()) {
        clauses.push("p.group_id = :group_id");
        params.push((":group_id", group_id.to_owned().into()));
    }
    let tag_ids = normalize_ids(&query.tag_ids);
    if !tag_ids.is_empty() {
        clauses.push(
            "EXISTS (SELECT 1 FROM profile_tags pt WHERE pt.profile_id = p.id
                     AND pt.tag_id IN (SELECT value FROM json_each(:tag_ids)))",
        );
        params.push((":tag_ids", json_string_array(&tag_ids).into()));
    }
    if let Some(keyword) = non_blank(query.keyword.as_deref()) {
        clauses.push(
            r"(p.name LIKE :keyword ESCAPE '\' OR p.note LIKE :keyword ESCAPE '\'
               OR CAST(p.fingerprint_seed AS TEXT) LIKE :keyword ESCAPE '\')",
        );
        params.push((":keyword", like_pattern(keyword).into()));
    }
    let where_sql = clauses.join(" AND ");

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM profiles p WHERE {where_sql}"),
        params.as_slice(),
        |r| r.get(0),
    )?;

    let order = match query.view {
        ProfileView::Trash => "p.deleted_at DESC, p.rowid DESC",
        ProfileView::All | ProfileView::Running => "p.created_at DESC, p.rowid DESC",
    };
    let sql = format!(
        "SELECT {}, {}, {}
         FROM profiles p
         LEFT JOIN groups g ON g.id = p.group_id
         LEFT JOIN proxies x ON x.id = p.proxy_id
         WHERE {where_sql}
         ORDER BY {order}
         LIMIT :limit OFFSET :offset",
        select_list(COLUMNS, "p", ""),
        select_list(groups::COLUMNS, "g", "g_"),
        select_list(proxies::COLUMNS, "x", "x_"),
    );
    params.push((":limit", i64::from(query.page_size).into()));
    params.push((
        ":offset",
        (i64::from(query.page - 1) * i64::from(query.page_size)).into(),
    ));

    let mut stmt = conn.prepare(&sql)?;
    let mut items = stmt
        .query_map(params.as_slice(), |row| {
            let group = match col::<Option<String>>(row, "g_", "id")? {
                Some(_) => Some(groups::from_row(row, "g_")?),
                None => None,
            };
            let proxy = match col::<Option<String>>(row, "x_", "id")? {
                Some(_) => Some(proxies::from_row(row, "x_")?),
                None => None,
            };
            Ok(ProfileRow {
                profile: from_row(row)?,
                group,
                proxy,
                tags: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let ids: Vec<String> = items.iter().map(|row| row.profile.id.clone()).collect();
    let mut tags_by_profile = load_tags(conn, &ids)?;
    for row in &mut items {
        row.tags = tags_by_profile.remove(&row.profile.id).unwrap_or_default();
        row.profile.draft.tag_ids = row.tags.iter().map(|tag| tag.id.clone()).collect();
    }

    Ok(Page {
        items,
        total: u64::try_from(total).unwrap_or_default(),
        page: query.page,
        page_size: query.page_size,
    })
}

/// Moves a profile to the trash. Already-trashed profiles keep their original `deleted_at`.
pub fn soft_delete(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE profiles SET deleted_at = COALESCE(deleted_at, ?2) WHERE id = ?1",
        params![id, now()],
    )?;
    ensure_found(changed, "profile", id)
}

/// Takes a profile out of the trash. No-op for profiles that are not in it.
pub fn restore(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("UPDATE profiles SET deleted_at = NULL WHERE id = ?1", [id])?;
    ensure_found(changed, "profile", id)
}

/// Deletes a profile row permanently; its tag links go with it (`ON DELETE CASCADE`). The
/// browser data directory is the caller's business.
pub fn purge(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
    ensure_found(changed, "profile", id)
}

/// Replaces the tag set of one profile.
pub fn set_tags(conn: &Connection, profile_id: &str, tag_ids: &[String]) -> AppResult<()> {
    let tag_ids = normalize_ids(tag_ids);
    in_tx(conn, |conn| {
        touch(conn, profile_id)?;
        replace_tags(conn, profile_id, &tag_ids)
    })
}

/// Adds tags to several profiles, keeping the tags they already have (batch action, R4.3).
/// All-or-nothing: an unknown profile or tag id fails the whole batch.
pub fn add_tags(conn: &Connection, profile_ids: &[String], tag_ids: &[String]) -> AppResult<()> {
    let tag_ids = normalize_ids(tag_ids);
    in_tx(conn, |conn| {
        let mut insert = conn.prepare(
            "INSERT INTO profile_tags (profile_id, tag_id) VALUES (?1, ?2)
             ON CONFLICT DO NOTHING",
        )?;
        for profile_id in profile_ids {
            touch(conn, profile_id)?;
            for tag_id in &tag_ids {
                insert.execute([profile_id, tag_id])?;
            }
        }
        Ok(())
    })
}

/// Moves several profiles into a group, or out of any group with `None` (batch action, R4.3).
/// All-or-nothing: an unknown profile or group id fails the whole batch.
pub fn set_group(
    conn: &Connection,
    profile_ids: &[String],
    group_id: Option<&str>,
) -> AppResult<()> {
    let now = now();
    in_tx(conn, |conn| {
        let mut update =
            conn.prepare("UPDATE profiles SET group_id = ?2, updated_at = ?3 WHERE id = ?1")?;
        for profile_id in profile_ids {
            let changed = update.execute(params![profile_id, group_id, now])?;
            ensure_found(changed, "profile", profile_id)?;
        }
        Ok(())
    })
}

/// Records a launch (design §4.2 step 6).
pub fn mark_launched(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE profiles SET last_launched_at = ?2 WHERE id = ?1",
        params![id, now()],
    )?;
    ensure_found(changed, "profile", id)
}

/// Bumps `updated_at`; `not_found` for an unknown profile.
fn touch(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE profiles SET updated_at = ?2 WHERE id = ?1",
        params![id, now()],
    )?;
    ensure_found(changed, "profile", id)
}

/// `tag_ids` must already be normalized (no duplicates).
fn replace_tags(conn: &Connection, profile_id: &str, tag_ids: &[String]) -> AppResult<()> {
    conn.execute(
        "DELETE FROM profile_tags WHERE profile_id = ?1",
        [profile_id],
    )?;
    let mut insert =
        conn.prepare("INSERT INTO profile_tags (profile_id, tag_id) VALUES (?1, ?2)")?;
    for tag_id in tag_ids {
        insert.execute([profile_id, tag_id])?;
    }
    Ok(())
}

/// Tags of the given profiles, each list ordered by tag name.
fn load_tags(conn: &Connection, profile_ids: &[String]) -> AppResult<HashMap<String, Vec<Tag>>> {
    let mut by_profile: HashMap<String, Vec<Tag>> = HashMap::new();
    if profile_ids.is_empty() {
        return Ok(by_profile);
    }
    let sql = format!(
        "SELECT pt.profile_id AS profile_id, {}
         FROM profile_tags pt JOIN tags t ON t.id = pt.tag_id
         WHERE pt.profile_id IN (SELECT value FROM json_each(?1))
         ORDER BY t.name",
        select_list(tags::COLUMNS, "t", "t_"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([json_string_array(profile_ids)], |row| {
        Ok((
            row.get::<_, String>("profile_id")?,
            tags::from_row(row, "t_")?,
        ))
    })?;
    for row in rows {
        let (profile_id, tag) = row?;
        by_profile.entry(profile_id).or_default().push(tag);
    }
    Ok(by_profile)
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// `%keyword%` with LIKE wildcards in the keyword escaped by `\`.
fn like_pattern(keyword: &str) -> String {
    let mut pattern = String::with_capacity(keyword.len() + 2);
    pattern.push('%');
    for c in keyword.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;
    use crate::error::ErrorCode;

    fn names(page: &Page<ProfileRow>) -> Vec<&str> {
        page.items
            .iter()
            .map(|row| row.profile.draft.name.as_str())
            .collect()
    }

    fn query(view: ProfileView) -> ProfileQuery {
        ProfileQuery {
            view,
            ..ProfileQuery::default()
        }
    }

    /// A draft with every field set.
    fn full_draft(db: &TestDb) -> ProfileDraft {
        ProfileDraft {
            name: "Shop 1".into(),
            note: Some("main account".into()),
            group_id: Some(db.group("Work").id),
            kernel_id: Some(db.kernel("k", 144).id),
            proxy_id: Some(db.proxy("127.0.0.1", 1080).id),
            tag_ids: vec![db.tag("b").id, db.tag("a").id],
            fingerprint_seed: Some(SEED_MAX),
            fp_platform: Some(FpPlatform::Macos),
            fp_platform_version: Some("15.2.0".into()),
            fp_brand: Some("Edge".into()),
            fp_brand_version: Some("150.0.0.0".into()),
            fp_hardware_concurrency: Some(8),
            fp_gpu_vendor: Some("NVIDIA Corporation".into()),
            fp_gpu_renderer: Some("NVIDIA GeForce RTX 3060".into()),
            disable_gpu_fingerprint: true,
            disable_spoofing: vec![SpoofingFeature::Gpu, SpoofingFeature::ClientRects],
            disable_non_proxied_udp: false,
            lang: Some("zh-CN".into()),
            accept_lang: Some("zh-CN,en-US".into()),
            timezone: Some("Asia/Shanghai".into()),
            headless: true,
            user_data_dir: Some(db.abs_path("udd")),
            start_urls: vec!["https://example.com".into(), "about:blank".into()],
            extra_args: Some("--no-sandbox --window-size=1280,800".into()),
        }
    }

    #[test]
    fn create_and_get_round_trip_every_field() {
        let db = TestDb::new();
        let draft = full_draft(&db);
        let created = create(&db.conn, draft.clone()).unwrap();

        let mut expected = draft;
        expected.tag_ids = {
            // Read back ordered by tag name: "a" before "b".
            let mut ids = expected.tag_ids.clone();
            ids.reverse();
            ids
        };
        expected.disable_spoofing = vec![SpoofingFeature::ClientRects, SpoofingFeature::Gpu];
        assert_eq!(created.draft, expected);
        assert_eq!(created.created_at, created.updated_at);
        assert_eq!(created.deleted_at, None);
        assert_eq!(created.last_launched_at, None);
        assert_eq!(get(&db.conn, &created.id).unwrap(), created);

        // Survives reopening the database (A2).
        let conn = db.reopen();
        assert_eq!(get(&conn, &created.id).unwrap(), created);

        let stored: (Option<String>, Option<String>) = db
            .conn
            .query_row(
                "SELECT disable_spoofing, start_urls FROM profiles WHERE id = ?1",
                [&created.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored.0.as_deref(), Some("clientrects,gpu"));
        assert_eq!(
            stored.1.as_deref(),
            Some(r#"["https://example.com","about:blank"]"#)
        );
    }

    #[test]
    fn create_normalizes_blank_and_list_fields() {
        let db = TestDb::new();
        let tag = db.tag("t");
        let created = create(
            &db.conn,
            ProfileDraft {
                name: "  p ".into(),
                note: Some(" ".into()),
                lang: Some("".into()),
                timezone: Some(" UTC ".into()),
                tag_ids: vec![tag.id.clone(), format!(" {} ", tag.id), "".into()],
                disable_spoofing: vec![SpoofingFeature::Font, SpoofingFeature::Font],
                start_urls: vec![" ".into(), " https://a.test ".into()],
                ..ProfileDraft::default()
            },
        )
        .unwrap();
        let d = &created.draft;
        assert_eq!(d.name, "p");
        assert_eq!((d.note.as_deref(), d.lang.as_deref()), (None, None));
        assert_eq!(d.timezone.as_deref(), Some("UTC"));
        assert_eq!(d.tag_ids, [tag.id]);
        assert_eq!(d.disable_spoofing, [SpoofingFeature::Font]);
        assert_eq!(d.start_urls, ["https://a.test"]);

        // Empty lists are stored as NULL, not "" or "[]".
        let plain = db.profile("plain");
        let stored: (Option<String>, Option<String>) = db
            .conn
            .query_row(
                "SELECT disable_spoofing, start_urls FROM profiles WHERE id = ?1",
                [&plain.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored, (None, None));
    }

    #[test]
    fn rejects_invalid_drafts() {
        let db = TestDb::new();
        let cases = [
            ProfileDraft {
                name: " ".into(),
                ..ProfileDraft::default()
            },
            ProfileDraft {
                fingerprint_seed: Some(0),
                ..TestDb::draft("p")
            },
            ProfileDraft {
                fingerprint_seed: Some(SEED_MAX + 1),
                ..TestDb::draft("p")
            },
            ProfileDraft {
                fp_hardware_concurrency: Some(0),
                ..TestDb::draft("p")
            },
            ProfileDraft {
                user_data_dir: Some("relative/dir".into()),
                ..TestDb::draft("p")
            },
            ProfileDraft {
                extra_args: Some("--window-name='unclosed".into()),
                ..TestDb::draft("p")
            },
            // Chromium would read it as a switch, bypassing the single-instance check.
            ProfileDraft {
                start_urls: vec![" --user-data-dir=/elsewhere ".into()],
                ..TestDb::draft("p")
            },
        ];
        for draft in cases {
            let err = create(&db.conn, draft.clone()).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{draft:?}");
        }
        create(
            &db.conn,
            ProfileDraft {
                fingerprint_seed: Some(1),
                ..TestDb::draft("p")
            },
        )
        .unwrap();
    }

    #[test]
    fn dangling_references_are_invalid_input() {
        let db = TestDb::new();
        let cases = [
            ProfileDraft {
                group_id: Some("missing".into()),
                ..TestDb::draft("p")
            },
            ProfileDraft {
                kernel_id: Some("missing".into()),
                ..TestDb::draft("p")
            },
            ProfileDraft {
                proxy_id: Some("missing".into()),
                ..TestDb::draft("p")
            },
            ProfileDraft {
                tag_ids: vec!["missing".into()],
                ..TestDb::draft("p")
            },
        ];
        for draft in cases {
            let err = create(&db.conn, draft).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{}", err.message);
        }
        // Nothing half-written: the failing tag insert rolled the profile back.
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM profiles", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn extra_args_are_validated_on_save() {
        let db = TestDb::new();
        let with_extra = |extra: &str| ProfileDraft {
            extra_args: Some(extra.into()),
            ..TestDb::draft("p")
        };
        // Valid quoting is stored verbatim (only trimmed), not re-joined.
        let text = r#"--window-name="My Shop" '--x=a b' --load-extension=C:\ext"#;
        let created = create(&db.conn, with_extra(&format!(" {text} "))).unwrap();
        assert_eq!(created.draft.extra_args.as_deref(), Some(text));

        // Open quotes, and overriding the profile's own user data directory.
        for bad in [
            r#"--a="b"#,
            "--a 'b",
            "--user-data-dir=/elsewhere",
            r#"--no-first-run "--User-Data-Dir=C:\my dir""#,
        ] {
            let err = update(&db.conn, &created.id, with_extra(bad)).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
            let err = create(&db.conn, with_extra(bad)).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
        }
        assert_eq!(get(&db.conn, &created.id).unwrap(), created);
    }

    #[test]
    fn partial_json_draft_takes_defaults() {
        let draft: ProfileDraft = serde_json::from_value(serde_json::json!({
            "name": "p",
            "fingerprintSeed": 1000,
            "disableSpoofing": ["clientrects", "gpu"],
        }))
        .unwrap();
        assert_eq!(draft.fingerprint_seed, Some(1000));
        assert!(draft.disable_non_proxied_udp);
        assert!(!draft.headless);
        assert_eq!(
            draft.disable_spoofing,
            [SpoofingFeature::ClientRects, SpoofingFeature::Gpu]
        );
        assert!(
            serde_json::from_value::<ProfileDraft>(serde_json::json!({ "fpPlatform": "android" }))
                .is_err()
        );
    }

    #[test]
    fn serializes_flat_camel_case() {
        let db = TestDb::new();
        let profile = create(&db.conn, full_draft(&db)).unwrap();
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["fingerprintSeed"], SEED_MAX);
        assert_eq!(json["fpPlatform"], "macos");
        assert_eq!(
            json["disableSpoofing"],
            serde_json::json!(["clientrects", "gpu"])
        );
        assert_eq!(json["disableNonProxiedUdp"], false);
        assert_eq!(json["deletedAt"], serde_json::Value::Null);
        assert!(json["createdAt"].is_string());
        assert!(json.get("draft").is_none());

        let page = list(&db.conn, &ProfileQuery::default(), &[]).unwrap();
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(json["total"], 1);
        assert_eq!(json["pageSize"], DEFAULT_PAGE_SIZE);
        let row = &json["items"][0];
        assert_eq!(row["name"], "Shop 1");
        assert_eq!(row["group"]["name"], "Work");
        assert_eq!(row["proxy"]["scheme"], "socks5");
        assert_eq!(row["tags"][0]["name"], "a");
        assert_eq!(row["tagIds"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn query_deserializes_with_defaults() {
        let q: ProfileQuery = serde_json::from_value(serde_json::json!({
            "view": "trash", "tagIds": ["t"]
        }))
        .unwrap();
        assert_eq!(q.page, 1);
        assert_eq!(q.page_size, DEFAULT_PAGE_SIZE);
        assert_eq!(q.view, ProfileView::Trash);
        assert_eq!(q.tag_ids, ["t"]);
    }

    #[test]
    fn update_replaces_fields_and_tags() {
        let db = TestDb::new();
        let created = create(&db.conn, full_draft(&db)).unwrap();
        let other_tag = db.tag("c");

        let updated = update(
            &db.conn,
            &created.id,
            ProfileDraft {
                tag_ids: vec![other_tag.id.clone()],
                fingerprint_seed: Some(2023),
                ..TestDb::draft("renamed")
            },
        )
        .unwrap();
        assert_eq!(updated.draft.name, "renamed");
        assert_eq!(updated.draft.fingerprint_seed, Some(2023));
        // Fields absent from the new draft are cleared, not merged.
        assert_eq!(updated.draft.group_id, None);
        assert_eq!(updated.draft.disable_spoofing, []);
        assert_eq!(updated.draft.tag_ids, [other_tag.id]);
        assert_eq!(updated.created_at, created.created_at);
        assert!(updated.updated_at >= created.updated_at);
        assert_eq!(get(&db.conn, &created.id).unwrap(), updated);

        let err = update(&db.conn, "missing", TestDb::draft("x")).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn pagination_edges() {
        let db = TestDb::new();
        for i in 1..=25 {
            db.profile(&format!("p{i:02}"));
        }
        let page = |page, page_size| {
            list(
                &db.conn,
                &ProfileQuery {
                    page,
                    page_size,
                    ..ProfileQuery::default()
                },
                &[],
            )
        };

        let first = page(1, 10).unwrap();
        assert_eq!(first.total, 25);
        assert_eq!((first.page, first.page_size), (1, 10));
        // Newest first; same-millisecond inserts fall back to insertion order.
        assert_eq!(names(&first)[..3], ["p25", "p24", "p23"]);
        assert_eq!(first.items.len(), 10);
        assert_eq!(page(2, 10).unwrap().items.len(), 10);
        let last = page(3, 10).unwrap();
        assert_eq!(names(&last), ["p05", "p04", "p03", "p02", "p01"]);
        let beyond = page(4, 10).unwrap();
        assert!(beyond.items.is_empty());
        assert_eq!(beyond.total, 25);
        assert_eq!(page(1, MAX_PAGE_SIZE).unwrap().items.len(), 25);
        assert_eq!(page(25, 1).unwrap().items.len(), 1);

        for (p, size) in [(0, 10), (1, 0), (1, MAX_PAGE_SIZE + 1)] {
            assert_eq!(
                page(p, size).unwrap_err().code,
                ErrorCode::InvalidInput,
                "page {p} size {size}"
            );
        }

        // Pages never overlap.
        let mut all: Vec<String> = (1..=3)
            .flat_map(|p| page(p, 10).unwrap().items)
            .map(|row| row.profile.id)
            .collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 25);
    }

    #[test]
    fn equal_timestamps_keep_newest_first() {
        let db = TestDb::new();
        let ids: Vec<String> = ["a", "b", "c"].map(|n| db.profile(n).id).into();
        // Force ties: without the rowid tiebreaker SQLite returns them oldest first.
        db.conn
            .execute_batch("UPDATE profiles SET created_at = '2026-01-01T00:00:00.000Z'")
            .unwrap();
        let all = list(&db.conn, &query(ProfileView::All), &[]).unwrap();
        assert_eq!(names(&all), ["c", "b", "a"]);

        for id in &ids {
            soft_delete(&db.conn, id).unwrap();
        }
        db.conn
            .execute_batch("UPDATE profiles SET deleted_at = '2026-01-02T00:00:00.000Z'")
            .unwrap();
        let trash = list(&db.conn, &query(ProfileView::Trash), &[]).unwrap();
        assert_eq!(names(&trash), ["c", "b", "a"]);
    }

    #[test]
    fn trash_view_soft_delete_and_restore() {
        let db = TestDb::new();
        let a = db.profile("a");
        let b = db.profile("b");
        let c = db.profile("c");

        soft_delete(&db.conn, &a.id).unwrap();
        soft_delete(&db.conn, &c.id).unwrap();
        let deleted_at = get(&db.conn, &a.id).unwrap().deleted_at.unwrap();
        // Deleting again keeps the original timestamp.
        soft_delete(&db.conn, &a.id).unwrap();
        assert_eq!(get(&db.conn, &a.id).unwrap().deleted_at, Some(deleted_at));

        let all = list(&db.conn, &query(ProfileView::All), &[]).unwrap();
        assert_eq!((names(&all), all.total), (vec!["b"], 1));
        let trash = list(&db.conn, &query(ProfileView::Trash), &[]).unwrap();
        assert_eq!((names(&trash), trash.total), (vec!["c", "a"], 2));
        assert!(
            trash
                .items
                .iter()
                .all(|row| row.profile.deleted_at.is_some())
        );

        restore(&db.conn, &a.id).unwrap();
        restore(&db.conn, &b.id).unwrap(); // not in the trash: no-op
        let all = list(&db.conn, &query(ProfileView::All), &[]).unwrap();
        assert_eq!(names(&all), ["b", "a"]);
        assert_eq!(get(&db.conn, &a.id).unwrap().deleted_at, None);

        for result in [
            soft_delete(&db.conn, "missing"),
            restore(&db.conn, "missing"),
            purge(&db.conn, "missing"),
        ] {
            assert_eq!(result.unwrap_err().code, ErrorCode::NotFound);
        }
    }

    #[test]
    fn running_view_filters_by_the_supplied_ids() {
        let db = TestDb::new();
        let a = db.profile("a");
        db.profile("b");
        let c = db.profile("c");
        soft_delete(&db.conn, &c.id).unwrap();
        let running = [a.id.clone(), c.id.clone(), "gone".to_owned()];

        let page = list(&db.conn, &query(ProfileView::Running), &running).unwrap();
        // The trashed profile is excluded even though it is in the set.
        assert_eq!((names(&page), page.total), (vec!["a"], 1));
        let page = list(&db.conn, &query(ProfileView::Running), &[]).unwrap();
        assert_eq!(page.total, 0);
        // Other views ignore the set.
        let page = list(&db.conn, &query(ProfileView::All), &running).unwrap();
        assert_eq!(page.total, 2);
    }

    #[test]
    fn group_tag_and_keyword_filters_combine() {
        let db = TestDb::new();
        let work = db.group("Work");
        let vip = db.tag("vip");
        let ads = db.tag("ads");
        let new = |name: &str, group: Option<&str>, tags: &[&str], seed: Option<u32>| {
            create(
                &db.conn,
                ProfileDraft {
                    group_id: group.map(Into::into),
                    tag_ids: tags.iter().map(|t| (*t).to_owned()).collect(),
                    fingerprint_seed: seed,
                    ..TestDb::draft(name)
                },
            )
            .unwrap()
        };
        new("Shop alpha", Some(&work.id), &[&vip.id], Some(1000));
        new("Shop beta", None, &[&vip.id, &ads.id], None);
        new("shop gamma", Some(&work.id), &[&ads.id], None);
        new("Forum", Some(&work.id), &[&vip.id], Some(2023));
        let mut noted = TestDb::draft("Mail");
        noted.note = Some("used for SHOP signups".into());
        create(&db.conn, noted).unwrap();

        let run = |group: Option<&str>, tags: &[&str], keyword: Option<&str>| {
            let page = list(
                &db.conn,
                &ProfileQuery {
                    group_id: group.map(Into::into),
                    tag_ids: tags.iter().map(|t| (*t).to_owned()).collect(),
                    keyword: keyword.map(Into::into),
                    ..ProfileQuery::default()
                },
                &[],
            )
            .unwrap();
            let mut names: Vec<String> = page
                .items
                .iter()
                .map(|row| row.profile.draft.name.clone())
                .collect();
            assert_eq!(page.total as usize, names.len());
            names.sort();
            names
        };

        // Keyword: name or note, ASCII case-insensitive.
        assert_eq!(
            run(None, &[], Some(" shop ")),
            ["Mail", "Shop alpha", "Shop beta", "shop gamma"]
        );
        // Keyword matches the seed as text.
        assert_eq!(run(None, &[], Some("202")), ["Forum"]);
        // Tags: any of the given tags.
        assert_eq!(
            run(None, &[&vip.id, &ads.id], None),
            ["Forum", "Shop alpha", "Shop beta", "shop gamma"]
        );
        // Keyword AND tag.
        assert_eq!(
            run(None, &[&vip.id], Some("shop")),
            ["Shop alpha", "Shop beta"]
        );
        // Group AND tag AND keyword.
        assert_eq!(
            run(Some(&work.id), &[&vip.id], Some("shop")),
            ["Shop alpha"]
        );
        assert_eq!(
            run(Some(&work.id), &[], None),
            ["Forum", "Shop alpha", "shop gamma"]
        );
        // Blank filters are ignored.
        assert_eq!(run(Some(" "), &[" "], Some("  ")).len(), 5);
        assert!(run(Some("missing"), &[], None).is_empty());
    }

    #[test]
    fn keyword_wildcards_match_literally() {
        let db = TestDb::new();
        db.profile("100% real");
        db.profile("1000 real");
        db.profile("a_b");
        db.profile("axb");
        db.profile(r"c\d");
        let run = |keyword: &str| {
            let page = list(
                &db.conn,
                &ProfileQuery {
                    keyword: Some(keyword.into()),
                    ..ProfileQuery::default()
                },
                &[],
            )
            .unwrap();
            names(&page)
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(run("0%"), ["100% real"]);
        assert_eq!(run("a_b"), ["a_b"]);
        assert_eq!(run(r"c\d"), [r"c\d"]);
    }

    #[test]
    fn rows_embed_group_proxy_and_tags() {
        let db = TestDb::new();
        let created = create(&db.conn, full_draft(&db)).unwrap();
        db.profile("bare");

        let page = list(&db.conn, &ProfileQuery::default(), &[]).unwrap();
        let bare = &page.items[0];
        assert_eq!(bare.profile.draft.name, "bare");
        assert_eq!((&bare.group, &bare.proxy), (&None, &None));
        assert!(bare.tags.is_empty());

        let full = &page.items[1];
        assert_eq!(full.profile, created);
        let group = full.group.as_ref().unwrap();
        assert_eq!(Some(&group.id), created.draft.group_id.as_ref());
        assert_eq!(group.name, "Work");
        let proxy = full.proxy.as_ref().unwrap();
        assert_eq!(Some(&proxy.id), created.draft.proxy_id.as_ref());
        assert_eq!((proxy.host.as_str(), proxy.port), ("127.0.0.1", 1080));
        let tag_names: Vec<&str> = full.tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(tag_names, ["a", "b"]);
    }

    #[test]
    fn purge_cascades_profile_tags() {
        let db = TestDb::new();
        let created = create(&db.conn, full_draft(&db)).unwrap();
        let links = || -> i64 {
            db.conn
                .query_row("SELECT COUNT(*) FROM profile_tags", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(links(), 2);

        purge(&db.conn, &created.id).unwrap();
        assert_eq!(links(), 0);
        assert_eq!(
            get(&db.conn, &created.id).unwrap_err().code,
            ErrorCode::NotFound
        );
        // The referenced group, proxy, kernel and tags are untouched.
        assert_eq!(crate::db::tags::list(&db.conn).unwrap().len(), 2);
        assert_eq!(crate::db::groups::list(&db.conn).unwrap().len(), 1);
    }

    #[test]
    fn batch_group_and_tags() {
        let db = TestDb::new();
        let work = db.group("Work");
        let vip = db.tag("vip");
        let ads = db.tag("ads");
        let a = db.profile("a");
        let b = create(
            &db.conn,
            ProfileDraft {
                tag_ids: vec![vip.id.clone()],
                ..TestDb::draft("b")
            },
        )
        .unwrap();
        let ids = [a.id.clone(), b.id.clone()];

        set_group(&db.conn, &ids, Some(&work.id)).unwrap();
        add_tags(&db.conn, &ids, &[vip.id.clone(), ads.id.clone()]).unwrap();
        for id in &ids {
            let p = get(&db.conn, id).unwrap();
            assert_eq!(p.draft.group_id.as_deref(), Some(work.id.as_str()));
            // Ordered by tag name: ads, vip.
            assert_eq!(p.draft.tag_ids, [ads.id.clone(), vip.id.clone()]);
        }

        set_tags(&db.conn, &a.id, std::slice::from_ref(&vip.id)).unwrap();
        assert_eq!(get(&db.conn, &a.id).unwrap().draft.tag_ids, [vip.id]);
        set_tags(&db.conn, &a.id, &[]).unwrap();
        assert!(get(&db.conn, &a.id).unwrap().draft.tag_ids.is_empty());

        set_group(&db.conn, &ids, None).unwrap();
        assert_eq!(get(&db.conn, &b.id).unwrap().draft.group_id, None);

        // All-or-nothing on unknown ids.
        let with_missing = [a.id.clone(), "missing".to_owned()];
        let err = set_group(&db.conn, &with_missing, Some(&work.id)).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(get(&db.conn, &a.id).unwrap().draft.group_id, None);
        let err = add_tags(&db.conn, &with_missing, std::slice::from_ref(&ads.id)).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(get(&db.conn, &a.id).unwrap().draft.tag_ids.is_empty());
        let err = set_group(&db.conn, &ids, Some("missing")).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        let err = add_tags(&db.conn, &ids, &["missing".to_owned()]).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        let err = set_tags(&db.conn, "missing", &[]).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn mark_launched_sets_timestamp() {
        let db = TestDb::new();
        let p = db.profile("p");
        mark_launched(&db.conn, &p.id).unwrap();
        let launched = get(&db.conn, &p.id).unwrap();
        assert!(launched.last_launched_at.unwrap() >= p.created_at);
        assert_eq!(
            mark_launched(&db.conn, "missing").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn corrupt_list_columns_surface_as_db_errors() {
        let db = TestDb::new();
        let p = db.profile("p");
        db.conn
            .execute(
                "UPDATE profiles SET disable_spoofing = 'font,webgl' WHERE id = ?1",
                [&p.id],
            )
            .unwrap();
        assert_eq!(get(&db.conn, &p.id).unwrap_err().code, ErrorCode::Db);
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern(r"a%b_c\d"), r"%a\%b\_c\\d%");
    }
}

//! Settings repository (design §2.2 `settings`): a key/value table with typed access.
//!
//! Only keys the user has changed are stored; anything absent reads as its default, so a
//! default can change in a later release without migrating rows.

use std::ops::RangeInclusive;

use rusqlite::{Connection, params};
use serde::{Deserialize, Deserializer, Serialize};

use super::{in_tx, require_absolute};
use crate::error::{AppError, AppResult};

text_enum! {
    /// UI language.
    pub enum Locale {
        ZhCn => "zh-CN",
        En => "en",
    }
}

text_enum! {
    pub enum Theme {
        Light => "light",
        Dark => "dark",
        System => "system",
    }
}

const USER_DATA_ROOT: &str = "user_data_root";
const LOCALE: &str = "locale";
const THEME: &str = "theme";
const BATCH_CONCURRENCY: &str = "batch_concurrency";
const BATCH_INTERVAL_MS: &str = "batch_interval_ms";
const SIDEBAR_COLLAPSED: &str = "sidebar_collapsed";

/// Bounds of the batch launch settings (R4.2 / R7.4).
pub const BATCH_CONCURRENCY_RANGE: RangeInclusive<u32> = 1..=32;
pub const BATCH_INTERVAL_MS_RANGE: RangeInclusive<u32> = 0..=60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Root of the default per-profile user-data-dirs. `None` = the app default
    /// (`<app_data_dir>/profiles`), resolved by the caller at runtime because it depends on
    /// the platform.
    pub user_data_root: Option<String>,
    pub locale: Locale,
    pub theme: Theme,
    /// Maximum number of profiles in `Starting` during a batch launch.
    pub batch_concurrency: u32,
    /// Delay before each spawn of a batch launch.
    pub batch_interval_ms: u32,
    pub sidebar_collapsed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            user_data_root: None,
            locale: Locale::ZhCn,
            theme: Theme::System,
            batch_concurrency: 3,
            batch_interval_ms: 800,
            sidebar_collapsed: true,
        }
    }
}

/// A partial update: absent fields stay unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(optional_fields))]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    /// Absent = unchanged, `null` (or blank) = back to the app default, string = custom root.
    #[serde(deserialize_with = "present")]
    pub user_data_root: Option<Option<String>>,
    pub locale: Option<Locale>,
    pub theme: Option<Theme>,
    pub batch_concurrency: Option<u32>,
    pub batch_interval_ms: Option<u32>,
    pub sidebar_collapsed: Option<bool>,
}

/// Distinguishes a field sent as `null` (`Some(None)`) from an absent one (`None`, via
/// `#[serde(default)]`).
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

pub fn get(conn: &Connection) -> AppResult<Settings> {
    let mut settings = Settings::default();
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (key, value) = row?;
        // A value that no longer parses or is out of range keeps its default rather than
        // failing startup. Unknown keys (e.g. from a newer build) are ignored.
        match key.as_str() {
            USER_DATA_ROOT => settings.user_data_root = Some(value),
            LOCALE => parse_into(&value, &mut settings.locale),
            THEME => parse_into(&value, &mut settings.theme),
            BATCH_CONCURRENCY => {
                parse_in_range(
                    &value,
                    &BATCH_CONCURRENCY_RANGE,
                    &mut settings.batch_concurrency,
                );
            }
            BATCH_INTERVAL_MS => {
                parse_in_range(
                    &value,
                    &BATCH_INTERVAL_MS_RANGE,
                    &mut settings.batch_interval_ms,
                );
            }
            SIDEBAR_COLLAPSED => parse_into(&value, &mut settings.sidebar_collapsed),
            _ => {}
        }
    }
    Ok(settings)
}

/// Applies `patch` atomically and returns the resulting settings.
pub fn update(conn: &Connection, patch: SettingsPatch) -> AppResult<Settings> {
    let user_data_root = patch
        .user_data_root
        .map(|root| root.map(|r| r.trim().to_owned()).filter(|r| !r.is_empty()));
    if let Some(Some(root)) = &user_data_root {
        require_absolute(root, "user data root")?;
    }
    check_range(
        "batch concurrency",
        patch.batch_concurrency,
        &BATCH_CONCURRENCY_RANGE,
    )?;
    check_range(
        "batch interval",
        patch.batch_interval_ms,
        &BATCH_INTERVAL_MS_RANGE,
    )?;

    in_tx(conn, |conn| {
        match user_data_root {
            Some(Some(root)) => put(conn, USER_DATA_ROOT, &root)?,
            Some(None) => {
                conn.execute("DELETE FROM settings WHERE key = ?1", [USER_DATA_ROOT])?;
            }
            None => {}
        }
        if let Some(locale) = patch.locale {
            put(conn, LOCALE, locale.as_str())?;
        }
        if let Some(theme) = patch.theme {
            put(conn, THEME, theme.as_str())?;
        }
        if let Some(n) = patch.batch_concurrency {
            put(conn, BATCH_CONCURRENCY, &n.to_string())?;
        }
        if let Some(ms) = patch.batch_interval_ms {
            put(conn, BATCH_INTERVAL_MS, &ms.to_string())?;
        }
        if let Some(collapsed) = patch.sidebar_collapsed {
            put(conn, SIDEBAR_COLLAPSED, &collapsed.to_string())?;
        }
        Ok(())
    })?;
    get(conn)
}

fn put(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn parse_into<T: std::str::FromStr>(value: &str, target: &mut T) {
    if let Ok(parsed) = value.parse() {
        *target = parsed;
    }
}

fn parse_in_range(value: &str, range: &RangeInclusive<u32>, target: &mut u32) {
    if let Ok(parsed) = value.parse::<u32>()
        && range.contains(&parsed)
    {
        *target = parsed;
    }
}

fn check_range(field: &str, value: Option<u32>, range: &RangeInclusive<u32>) -> AppResult<()> {
    match value {
        Some(v) if !range.contains(&v) => Err(AppError::invalid_input(format!(
            "{field} must be {}-{}",
            range.start(),
            range.end()
        ))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;
    use crate::error::ErrorCode;

    #[test]
    fn defaults_when_unset() {
        let db = TestDb::new();
        let settings = get(&db.conn).unwrap();
        assert_eq!(
            settings,
            Settings {
                user_data_root: None,
                locale: Locale::ZhCn,
                theme: Theme::System,
                batch_concurrency: 3,
                batch_interval_ms: 800,
                sidebar_collapsed: true,
            }
        );
        assert_eq!(settings, Settings::default());
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "userDataRoot": null,
                "locale": "zh-CN",
                "theme": "system",
                "batchConcurrency": 3,
                "batchIntervalMs": 800,
                "sidebarCollapsed": true,
            })
        );
    }

    #[test]
    fn patch_changes_only_given_keys_and_persists() {
        let db = TestDb::new();
        let root = db.abs_path("browser-data");
        let updated = update(
            &db.conn,
            SettingsPatch {
                user_data_root: Some(Some(format!(" {root} "))),
                locale: Some(Locale::En),
                batch_concurrency: Some(5),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        let expected = Settings {
            user_data_root: Some(root),
            locale: Locale::En,
            batch_concurrency: 5,
            ..Settings::default()
        };
        assert_eq!(updated, expected);

        let updated = update(
            &db.conn,
            SettingsPatch {
                theme: Some(Theme::Dark),
                batch_interval_ms: Some(0),
                sidebar_collapsed: Some(false),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        let expected = Settings {
            theme: Theme::Dark,
            batch_interval_ms: 0,
            sidebar_collapsed: false,
            ..expected
        };
        assert_eq!(updated, expected);
        assert_eq!(get(&db.reopen()).unwrap(), expected);

        // Empty patch: nothing changes.
        assert_eq!(
            update(&db.conn, SettingsPatch::default()).unwrap(),
            expected
        );
    }

    #[test]
    fn user_data_root_can_be_reset() {
        let db = TestDb::new();
        let set = |root: Option<&str>| {
            update(
                &db.conn,
                SettingsPatch {
                    user_data_root: Some(root.map(Into::into)),
                    ..SettingsPatch::default()
                },
            )
        };
        set(Some(&db.abs_path("x"))).unwrap();
        assert_eq!(set(None).unwrap().user_data_root, None);
        set(Some(&db.abs_path("x"))).unwrap();
        assert_eq!(set(Some("  ")).unwrap().user_data_root, None);
        assert_eq!(
            set(Some("relative")).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn rejects_out_of_range_values_atomically() {
        let db = TestDb::new();
        for patch in [
            SettingsPatch {
                batch_concurrency: Some(0),
                ..SettingsPatch::default()
            },
            SettingsPatch {
                batch_concurrency: Some(33),
                locale: Some(Locale::En),
                ..SettingsPatch::default()
            },
            SettingsPatch {
                batch_interval_ms: Some(60_001),
                ..SettingsPatch::default()
            },
        ] {
            assert_eq!(
                update(&db.conn, patch).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
        assert_eq!(get(&db.conn).unwrap(), Settings::default());
    }

    #[test]
    fn unreadable_stored_values_fall_back_to_defaults() {
        let db = TestDb::new();
        db.conn
            .execute_batch(
                "INSERT INTO settings (key, value) VALUES
                   ('locale', 'fr'), ('theme', 'dark'), ('batch_concurrency', '0'),
                   ('batch_interval_ms', 'soon'), ('sidebar_collapsed', 'false'),
                   ('from_the_future', '1');",
            )
            .unwrap();
        let settings = get(&db.conn).unwrap();
        assert_eq!(
            settings,
            Settings {
                theme: Theme::Dark,
                sidebar_collapsed: false,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn patch_distinguishes_null_from_absent() {
        let parse = |v: serde_json::Value| serde_json::from_value::<SettingsPatch>(v).unwrap();
        assert_eq!(parse(serde_json::json!({})).user_data_root, None);
        assert_eq!(
            parse(serde_json::json!({ "userDataRoot": null })).user_data_root,
            Some(None)
        );
        let patch = parse(serde_json::json!({ "userDataRoot": "/data", "locale": "en" }));
        assert_eq!(patch.user_data_root, Some(Some("/data".to_owned())));
        assert_eq!(patch.locale, Some(Locale::En));
        assert!(
            serde_json::from_value::<SettingsPatch>(serde_json::json!({ "theme": "blue" }))
                .is_err()
        );
    }
}

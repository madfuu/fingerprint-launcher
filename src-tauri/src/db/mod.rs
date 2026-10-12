//! SQLite persistence (design §2): connection setup, migrations and one repository module per
//! table.
//!
//! Repositories are free functions over `&Connection`. They compose inside [`in_tx`] and can
//! run on a blocking thread while the caller holds the connection lock (design §4.1).

mod migrate;
#[macro_use]
mod text_enum;

pub mod groups;
pub mod kernels;
pub mod profiles;
pub mod proxies;
pub mod settings;
pub mod tags;

#[cfg(test)]
pub(crate) mod test_support;

use std::path::Path;

use chrono::{SecondsFormat, Utc};
use rusqlite::types::FromSql;
use rusqlite::{Connection, Row};
use serde::Serialize;

use crate::error::{AppError, AppResult};

pub use migrate::{SCHEMA_VERSION, schema_version};

/// File name of the database inside the app data directory (design §2.1).
pub const DB_FILE_NAME: &str = "launcher.db";

/// Opens (creating if needed) the database at `path`, configures the connection and applies
/// pending migrations.
pub fn open(path: &Path) -> AppResult<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut conn = Connection::open(path)?;
    // Per connection: SQLite ships with foreign keys off, and every `ON DELETE` clause in the
    // schema depends on them. The pragma is a no-op inside a transaction, so it goes first.
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate::run(&mut conn)?;
    Ok(conn)
}

/// Runs `f` atomically: in a new transaction, or in a savepoint when the caller already has a
/// transaction open, so repository functions stay composable (e.g. an import that creates many
/// profiles at once). Either way an error (or panic) in `f` undoes exactly the writes of `f`;
/// an outer caller that recovers from that error keeps its own writes and no half-written
/// record.
pub fn in_tx<T>(conn: &Connection, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
    if conn.is_autocommit() {
        let tx = conn.unchecked_transaction()?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    } else {
        let savepoint = NestedSavepoint::begin(conn)?;
        let value = f(conn)?;
        savepoint.release()?;
        Ok(value)
    }
}

/// A `SAVEPOINT` inside an open transaction, rolled back on drop unless released. rusqlite's own
/// `Savepoint` needs `&mut Connection`, which repository functions do not have.
struct NestedSavepoint<'c> {
    conn: &'c Connection,
    released: bool,
}

impl<'c> NestedSavepoint<'c> {
    /// Savepoint names may repeat; `RELEASE` / `ROLLBACK TO` act on the innermost one.
    const NAME: &'static str = "in_tx";

    fn begin(conn: &'c Connection) -> AppResult<Self> {
        conn.execute_batch(&format!("SAVEPOINT {}", Self::NAME))?;
        Ok(Self {
            conn,
            released: false,
        })
    }

    fn release(mut self) -> AppResult<()> {
        self.conn
            .execute_batch(&format!("RELEASE {}", Self::NAME))?;
        self.released = true;
        Ok(())
    }
}

impl Drop for NestedSavepoint<'_> {
    fn drop(&mut self) {
        if !self.released {
            // Fails only when SQLite already rolled back the whole transaction (e.g. disk full),
            // in which case there is nothing left to undo.
            let _ = self.conn.execute_batch(&format!(
                "ROLLBACK TO {name}; RELEASE {name}",
                name = Self::NAME
            ));
        }
    }
}

/// One page of a paginated list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Number of matching rows across all pages.
    pub total: u64,
    /// 1-based.
    pub page: u32,
    pub page_size: u32,
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Current time as RFC 3339 UTC with fixed millisecond precision, so the text sorts
/// chronologically.
pub(crate) fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `alias.col AS {prefix}col, ...`, so rows of joined tables can be read by name through
/// [`col`] without clashing.
pub(crate) fn select_list(columns: &[&str], alias: &str, prefix: &str) -> String {
    columns
        .iter()
        .map(|c| format!("{alias}.{c} AS {prefix}{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Reads the column `{prefix}{name}` produced by [`select_list`].
pub(crate) fn col<T: FromSql>(row: &Row<'_>, prefix: &str, name: &str) -> rusqlite::Result<T> {
    row.get(format!("{prefix}{name}").as_str())
}

/// Maps "no row changed" to `not_found`.
pub(crate) fn ensure_found(changed: usize, what: &str, id: &str) -> AppResult<()> {
    if changed == 0 {
        Err(not_found(what, id))
    } else {
        Ok(())
    }
}

pub(crate) fn not_found(what: &str, id: &str) -> AppError {
    AppError::not_found(format!("{what} {id} not found"))
}

/// Trims a required text field; blank is rejected.
pub(crate) fn required_text(value: &str, field: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::invalid_input(format!("{field} is required")));
    }
    Ok(value.to_owned())
}

/// Trims an optional text field; blank becomes `None` so it is stored as NULL.
pub(crate) fn clean_opt(value: &mut Option<String>) {
    *value = value
        .take()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
}

/// Paths are stored absolute: a relative one would resolve against whatever the working
/// directory of the launcher happens to be.
pub(crate) fn require_absolute(path: &str, field: &str) -> AppResult<()> {
    if Path::new(path).is_absolute() {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!(
            "{field} must be an absolute path: {path}"
        )))
    }
}

/// Colors are `#rgb` or `#rrggbb`; the frontend puts them straight into styles.
pub(crate) fn validate_color(color: &str) -> AppResult<()> {
    let ok = color.strip_prefix('#').is_some_and(|hex| {
        matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit())
    });
    if ok {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!(
            "color must be #rgb or #rrggbb: {color}"
        )))
    }
}

/// JSON array of strings, the form taken by `json_each(?)` and the `start_urls` column.
pub(crate) fn json_string_array(values: &[String]) -> String {
    serde_json::Value::Array(
        values
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::test_support::TestDb;
    use super::*;

    #[test]
    fn connection_enables_foreign_keys_and_wal() {
        let db = TestDb::new();
        let fk: bool = db
            .conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        let mode: String = db
            .conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert!(fk);
        assert_eq!(mode, "wal");
    }

    #[test]
    fn open_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("data").join(DB_FILE_NAME);
        let conn = open(&path).unwrap();
        drop(conn);
        assert!(path.is_file());
    }

    #[test]
    fn in_tx_rolls_back_on_error_and_joins_outer_transaction() {
        let db = TestDb::new();
        let insert = |conn: &Connection, name: &str| -> AppResult<()> {
            conn.execute(
                "INSERT INTO tags (id, name, color, created_at) VALUES (?1, ?1, '#fff', ?2)",
                (name, now()),
            )?;
            Ok(())
        };
        let count = || -> i64 {
            db.conn
                .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
                .unwrap()
        };

        let result: AppResult<()> = in_tx(&db.conn, |conn| {
            insert(conn, "a")?;
            Err(AppError::invalid_input("abort"))
        });
        assert!(result.is_err());
        assert_eq!(count(), 0);

        in_tx(&db.conn, |conn| {
            insert(conn, "b")?;
            // Nested call runs inside the outer transaction instead of failing on BEGIN.
            in_tx(conn, |conn| insert(conn, "c"))
        })
        .unwrap();
        assert_eq!(count(), 2);
        assert!(db.conn.is_autocommit());
    }

    #[test]
    fn nested_failure_is_undone_when_the_outer_call_recovers() {
        let db = TestDb::new();
        let insert = |conn: &Connection, name: &str| -> AppResult<()> {
            conn.execute(
                "INSERT INTO tags (id, name, color, created_at) VALUES (?1, ?1, '#fff', ?2)",
                (name, now()),
            )?;
            Ok(())
        };

        in_tx(&db.conn, |conn| {
            insert(conn, "a")?;
            let nested: AppResult<()> = in_tx(conn, |conn| {
                insert(conn, "half-written")?;
                Err(AppError::invalid_input("abort"))
            });
            assert!(nested.is_err());
            // The outer transaction is still usable after the nested rollback.
            in_tx(conn, |conn| insert(conn, "b"))
        })
        .unwrap();

        let names: Vec<String> = db
            .conn
            .prepare("SELECT name FROM tags ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(names, ["a", "b"]);
        assert!(db.conn.is_autocommit());
    }

    #[test]
    fn text_helpers() {
        assert_eq!(required_text("  x ", "name").unwrap(), "x");
        assert_eq!(
            required_text("  ", "name").unwrap_err().code,
            crate::error::ErrorCode::InvalidInput
        );
        let mut v = Some("  ".to_owned());
        clean_opt(&mut v);
        assert_eq!(v, None);
        let mut v = Some(" a b ".to_owned());
        clean_opt(&mut v);
        assert_eq!(v.as_deref(), Some("a b"));

        for ok in ["#fff", "#3B82F6"] {
            validate_color(ok).unwrap();
        }
        for bad in ["", "fff", "#ffff", "#ggg", "blue", "#3b82f6;x"] {
            assert!(validate_color(bad).is_err(), "{bad}");
        }
        assert_eq!(
            json_string_array(&["a\"b".to_owned(), "c".to_owned()]),
            r#"["a\"b","c"]"#
        );
    }
}

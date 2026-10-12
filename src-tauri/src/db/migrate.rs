//! Versioned schema migrations. `meta.schema_version` records how many entries of
//! [`MIGRATIONS`] have been applied; each one runs in its own transaction together with the
//! version bump, so a failed migration leaves the previous version intact.

use rusqlite::{Connection, OptionalExtension};

use crate::error::{AppError, AppResult};

/// Index `i` upgrades the schema from version `i` to `i + 1`.
const MIGRATIONS: &[&str] = &[include_str!("migrations/v1.sql")];

/// Schema version this build creates and understands.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub(super) fn run(conn: &mut Connection) -> AppResult<()> {
    let current = schema_version(conn)?;
    if current > SCHEMA_VERSION {
        return Err(AppError::db(format!(
            "database schema v{current} is newer than this build supports (v{SCHEMA_VERSION})"
        )));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [(index + 1).to_string()],
        )?;
        tx.commit()?;
    }
    Ok(())
}

/// Applied schema version; 0 for an empty database. `meta` itself is created by v1.
pub fn schema_version(conn: &Connection) -> AppResult<u32> {
    let has_meta: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
        [],
        |r| r.get(0),
    )?;
    if !has_meta {
        return Ok(0);
    }
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match value {
        None => Ok(0),
        Some(v) => v
            .parse()
            .map_err(|_| AppError::db(format!("invalid schema_version {v:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;
    use crate::db::{self, groups};
    use crate::error::ErrorCode;

    #[test]
    fn fresh_database_is_at_latest_version() {
        let db = TestDb::new();
        assert_eq!(SCHEMA_VERSION, 1);
        assert_eq!(schema_version(&db.conn).unwrap(), SCHEMA_VERSION);
        let tables: Vec<String> = db
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            tables,
            [
                "groups",
                "kernels",
                "meta",
                "profile_tags",
                "profiles",
                "proxies",
                "settings",
                "tags"
            ]
        );
    }

    #[test]
    fn migrations_are_idempotent_and_keep_data() {
        let mut db = TestDb::new();
        let group = db.group("Work");

        // Re-running on the open connection is a no-op.
        run(&mut db.conn).unwrap();
        // So is reopening the file, which runs them again.
        let conn = db.reopen();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(groups::get(&conn, &group.id).unwrap(), group);
    }

    #[test]
    fn refuses_a_newer_schema() {
        let db = TestDb::new();
        db.conn
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                [(SCHEMA_VERSION + 1).to_string()],
            )
            .unwrap();
        let err = db::open(&db.path()).unwrap_err();
        assert_eq!(err.code, ErrorCode::Db);
        assert!(err.message.contains("newer"), "{}", err.message);
    }
}

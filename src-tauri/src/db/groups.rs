//! Group repository (design §2.2 `groups`). A profile belongs to at most one group; deleting a
//! group leaves its profiles ungrouped (`ON DELETE SET NULL`).

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{
    clean_opt, col, ensure_found, new_id, not_found, now, required_text, select_list,
    validate_color,
};
use crate::error::AppResult;

pub(crate) const COLUMNS: &[&str] = &["id", "name", "color", "sort_order", "created_at"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    /// `#rgb` / `#rrggbb`.
    pub color: Option<String>,
    pub sort_order: i64,
    pub created_at: String,
}

/// Editable fields of a group.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GroupInput {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub sort_order: i64,
}

impl GroupInput {
    fn normalized(mut self) -> AppResult<Self> {
        self.name = required_text(&self.name, "group name")?;
        clean_opt(&mut self.color);
        if let Some(color) = &self.color {
            validate_color(color)?;
        }
        Ok(self)
    }
}

/// Reads a group from columns selected with `select_list(COLUMNS, alias, prefix)`.
pub(crate) fn from_row(row: &Row<'_>, prefix: &str) -> rusqlite::Result<Group> {
    Ok(Group {
        id: col(row, prefix, "id")?,
        name: col(row, prefix, "name")?,
        color: col(row, prefix, "color")?,
        sort_order: col(row, prefix, "sort_order")?,
        created_at: col(row, prefix, "created_at")?,
    })
}

fn select(where_order: &str) -> String {
    format!(
        "SELECT {} FROM groups g {where_order}",
        select_list(COLUMNS, "g", "")
    )
}

/// All groups by `sort_order`, then creation order.
pub fn list(conn: &Connection) -> AppResult<Vec<Group>> {
    let mut stmt = conn.prepare(&select("ORDER BY g.sort_order, g.created_at, g.rowid"))?;
    let rows = stmt.query_map([], |row| from_row(row, ""))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Group> {
    conn.query_row(&select("WHERE g.id = ?1"), [id], |row| from_row(row, ""))
        .optional()?
        .ok_or_else(|| not_found("group", id))
}

/// The group named `name`, compared like the unique index (trimmed, ASCII case-insensitive).
pub fn find_by_name(conn: &Connection, name: &str) -> AppResult<Option<Group>> {
    Ok(conn
        .query_row(&select("WHERE g.name = ?1"), [name.trim()], |row| {
            from_row(row, "")
        })
        .optional()?)
}

pub fn create(conn: &Connection, input: GroupInput) -> AppResult<Group> {
    let input = input.normalized()?;
    let id = new_id();
    conn.execute(
        "INSERT INTO groups (id, name, color, sort_order, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, input.name, input.color, input.sort_order, now()],
    )?;
    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, input: GroupInput) -> AppResult<Group> {
    let input = input.normalized()?;
    let changed = conn.execute(
        "UPDATE groups SET name = ?2, color = ?3, sort_order = ?4 WHERE id = ?1",
        params![id, input.name, input.color, input.sort_order],
    )?;
    ensure_found(changed, "group", id)?;
    get(conn, id)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM groups WHERE id = ?1", [id])?;
    ensure_found(changed, "group", id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profiles;
    use crate::db::test_support::TestDb;
    use crate::error::ErrorCode;

    fn input(name: &str, color: Option<&str>, sort_order: i64) -> GroupInput {
        GroupInput {
            name: name.into(),
            color: color.map(Into::into),
            sort_order,
        }
    }

    #[test]
    fn crud_and_ordering() {
        let db = TestDb::new();
        let b = create(&db.conn, input("B", Some("#3b82f6"), 1)).unwrap();
        let a = create(&db.conn, input(" A ", None, 0)).unwrap();
        let c = create(&db.conn, input("C", Some(" "), 1)).unwrap();
        assert_eq!(a.name, "A");
        assert_eq!(c.color, None);
        let names = |conn| {
            list(conn)
                .unwrap()
                .into_iter()
                .map(|g| g.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&db.conn), ["A", "B", "C"]);

        let b2 = update(&db.conn, &b.id, input("B2", Some("#fff"), -1)).unwrap();
        assert_eq!(b2.color.as_deref(), Some("#fff"));
        assert_eq!(b2.created_at, b.created_at);
        assert_eq!(names(&db.conn), ["B2", "A", "C"]);

        delete(&db.conn, &a.id).unwrap();
        assert_eq!(names(&db.conn), ["B2", "C"]);
        assert_eq!(get(&db.conn, &a.id).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(
            delete(&db.conn, &a.id).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            update(&db.conn, &a.id, input("A", None, 0))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn rejects_duplicate_names_and_bad_input() {
        let db = TestDb::new();
        let a = db.group("Work");
        let code = |r: AppResult<Group>| r.unwrap_err().code;
        assert_eq!(
            code(create(&db.conn, input("Work", None, 0))),
            ErrorCode::Conflict
        );
        // Names are unique case-insensitively.
        assert_eq!(
            code(create(&db.conn, input("work", None, 0))),
            ErrorCode::Conflict
        );
        let other = db.group("Home");
        assert_eq!(
            code(update(&db.conn, &other.id, input("Work", None, 0))),
            ErrorCode::Conflict
        );
        assert_eq!(
            code(create(&db.conn, input("  ", None, 0))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(&db.conn, input("X", Some("red"), 0))),
            ErrorCode::InvalidInput
        );
        // Renaming to its own name, or changing only its case, is not a conflict.
        update(&db.conn, &a.id, input("Work", Some("#000"), 0)).unwrap();
        update(&db.conn, &a.id, input("WORK", Some("#000"), 0)).unwrap();
    }

    #[test]
    fn find_by_name_ignores_ascii_case() {
        let db = TestDb::new();
        let work = db.group("Work");
        assert_eq!(find_by_name(&db.conn, " work ").unwrap(), Some(work));
        assert_eq!(find_by_name(&db.conn, "Home").unwrap(), None);
    }

    #[test]
    fn deleting_a_group_ungroups_its_profiles() {
        let db = TestDb::new();
        let group = db.group("Work");
        let mut draft = TestDb::draft("p");
        draft.group_id = Some(group.id.clone());
        let profile = profiles::create(&db.conn, draft).unwrap();
        assert_eq!(profile.draft.group_id.as_deref(), Some(group.id.as_str()));

        delete(&db.conn, &group.id).unwrap();
        let profile = profiles::get(&db.conn, &profile.id).unwrap();
        assert_eq!(profile.draft.group_id, None);
    }
}

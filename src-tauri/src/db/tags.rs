//! Tag repository (design §2.2 `tags`). Tags attach to profiles many-to-many through
//! `profile_tags` (managed in [`super::profiles`]); deleting a tag removes its links
//! (`ON DELETE CASCADE`).

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{
    col, ensure_found, new_id, not_found, now, required_text, select_list, validate_color,
};
use crate::error::AppResult;

pub(crate) const COLUMNS: &[&str] = &["id", "name", "color", "created_at"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    /// `#rgb` / `#rrggbb`.
    pub color: String,
    pub created_at: String,
}

/// Editable fields of a tag.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInput {
    pub name: String,
    pub color: String,
}

impl TagInput {
    fn normalized(mut self) -> AppResult<Self> {
        self.name = required_text(&self.name, "tag name")?;
        self.color = self.color.trim().to_owned();
        validate_color(&self.color)?;
        Ok(self)
    }
}

/// Reads a tag from columns selected with `select_list(COLUMNS, alias, prefix)`.
pub(crate) fn from_row(row: &Row<'_>, prefix: &str) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: col(row, prefix, "id")?,
        name: col(row, prefix, "name")?,
        color: col(row, prefix, "color")?,
        created_at: col(row, prefix, "created_at")?,
    })
}

fn select(where_order: &str) -> String {
    format!(
        "SELECT {} FROM tags t {where_order}",
        select_list(COLUMNS, "t", "")
    )
}

/// All tags by name.
pub fn list(conn: &Connection) -> AppResult<Vec<Tag>> {
    let mut stmt = conn.prepare(&select("ORDER BY t.name"))?;
    let rows = stmt.query_map([], |row| from_row(row, ""))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Tag> {
    conn.query_row(&select("WHERE t.id = ?1"), [id], |row| from_row(row, ""))
        .optional()?
        .ok_or_else(|| not_found("tag", id))
}

pub fn create(conn: &Connection, input: TagInput) -> AppResult<Tag> {
    let input = input.normalized()?;
    let id = new_id();
    conn.execute(
        "INSERT INTO tags (id, name, color, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, input.name, input.color, now()],
    )?;
    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, input: TagInput) -> AppResult<Tag> {
    let input = input.normalized()?;
    let changed = conn.execute(
        "UPDATE tags SET name = ?2, color = ?3 WHERE id = ?1",
        params![id, input.name, input.color],
    )?;
    ensure_found(changed, "tag", id)?;
    get(conn, id)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM tags WHERE id = ?1", [id])?;
    ensure_found(changed, "tag", id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profiles;
    use crate::db::test_support::TestDb;
    use crate::error::ErrorCode;

    fn input(name: &str, color: &str) -> TagInput {
        TagInput {
            name: name.into(),
            color: color.into(),
        }
    }

    #[test]
    fn crud_ordered_by_name() {
        let db = TestDb::new();
        let vip = create(&db.conn, input("vip", " #f00 ")).unwrap();
        create(&db.conn, input("ads", "#00ff00")).unwrap();
        assert_eq!(vip.color, "#f00");
        let names = |conn| {
            list(conn)
                .unwrap()
                .into_iter()
                .map(|t| t.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&db.conn), ["ads", "vip"]);

        let renamed = update(&db.conn, &vip.id, input("aaa", "#123")).unwrap();
        assert_eq!(renamed.created_at, vip.created_at);
        assert_eq!(names(&db.conn), ["aaa", "ads"]);

        delete(&db.conn, &vip.id).unwrap();
        assert_eq!(names(&db.conn), ["ads"]);
        assert_eq!(
            get(&db.conn, &vip.id).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn rejects_duplicate_names_and_bad_input() {
        let db = TestDb::new();
        db.tag("vip");
        let code = |r: AppResult<Tag>| r.unwrap_err().code;
        assert_eq!(
            code(create(&db.conn, input("vip", "#fff"))),
            ErrorCode::Conflict
        );
        // Names are unique case-insensitively.
        assert_eq!(
            code(create(&db.conn, input("VIP", "#fff"))),
            ErrorCode::Conflict
        );
        assert_eq!(
            code(create(&db.conn, input(" ", "#fff"))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(&db.conn, input("x", ""))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(update(&db.conn, "missing", input("x", "#fff"))),
            ErrorCode::NotFound
        );
    }

    #[test]
    fn deleting_a_tag_removes_its_profile_links() {
        let db = TestDb::new();
        let vip = db.tag("vip");
        let ads = db.tag("ads");
        let mut draft = TestDb::draft("p");
        draft.tag_ids = vec![vip.id.clone(), ads.id.clone()];
        let profile = profiles::create(&db.conn, draft).unwrap();

        delete(&db.conn, &vip.id).unwrap();
        assert_eq!(
            profiles::get(&db.conn, &profile.id).unwrap().draft.tag_ids,
            [ads.id]
        );
        let links: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM profile_tags WHERE tag_id = ?1",
                [&vip.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(links, 0);
    }
}

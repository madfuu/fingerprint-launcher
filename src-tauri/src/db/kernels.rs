//! Kernel repository: registered fingerprint-chromium executables (design §2.2 `kernels`).
//!
//! Invariant: while any kernel exists, exactly one is the default. The first kernel added
//! becomes the default, and deleting the default promotes the oldest remaining one.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{
    col, ensure_found, in_tx, new_id, not_found, now, require_absolute, required_text, select_list,
};
use crate::error::{AppError, AppResult};

const COLUMNS: &[&str] = &[
    "id",
    "name",
    "exec_path",
    "major_version",
    "full_version",
    "is_default",
    "created_at",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kernel {
    pub id: String,
    pub name: String,
    pub exec_path: String,
    /// Chromium major version; selects the supported argument set (design §3.2).
    pub major_version: u32,
    /// Display only, e.g. `150.0.7871.186`.
    pub full_version: Option<String>,
    pub is_default: bool,
    pub created_at: String,
}

/// Editable fields of a kernel.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelInput {
    pub name: String,
    pub exec_path: String,
    pub major_version: u32,
    #[serde(default)]
    pub full_version: Option<String>,
}

impl KernelInput {
    fn normalized(mut self) -> AppResult<Self> {
        self.name = required_text(&self.name, "kernel name")?;
        self.exec_path = required_text(&self.exec_path, "kernel path")?;
        require_absolute(&self.exec_path, "kernel path")?;
        if self.major_version == 0 {
            return Err(AppError::invalid_input(
                "kernel major version must be positive",
            ));
        }
        super::clean_opt(&mut self.full_version);
        Ok(self)
    }
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Kernel> {
    Ok(Kernel {
        id: col(row, "", "id")?,
        name: col(row, "", "name")?,
        exec_path: col(row, "", "exec_path")?,
        major_version: col(row, "", "major_version")?,
        full_version: col(row, "", "full_version")?,
        is_default: col(row, "", "is_default")?,
        created_at: col(row, "", "created_at")?,
    })
}

fn select(where_order: &str) -> String {
    format!(
        "SELECT {} FROM kernels k {where_order}",
        select_list(COLUMNS, "k", "")
    )
}

/// All kernels, oldest first.
pub fn list(conn: &Connection) -> AppResult<Vec<Kernel>> {
    let mut stmt = conn.prepare(&select("ORDER BY k.created_at, k.rowid"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Kernel> {
    conn.query_row(&select("WHERE k.id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| not_found("kernel", id))
}

/// The kernel used by profiles whose `kernel_id` is NULL; `None` when no kernel is configured.
pub fn get_default(conn: &Connection) -> AppResult<Option<Kernel>> {
    Ok(conn
        .query_row(&select("WHERE k.is_default = 1"), [], from_row)
        .optional()?)
}

pub fn create(conn: &Connection, input: KernelInput) -> AppResult<Kernel> {
    let input = input.normalized()?;
    let id = new_id();
    in_tx(conn, |conn| {
        let has_default: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM kernels WHERE is_default = 1)",
            [],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO kernels
               (id, name, exec_path, major_version, full_version, is_default, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                input.name,
                input.exec_path,
                input.major_version,
                input.full_version,
                !has_default,
                now()
            ],
        )?;
        Ok(())
    })?;
    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, input: KernelInput) -> AppResult<Kernel> {
    let input = input.normalized()?;
    let changed = conn.execute(
        "UPDATE kernels SET name = ?2, exec_path = ?3, major_version = ?4, full_version = ?5
         WHERE id = ?1",
        params![
            id,
            input.name,
            input.exec_path,
            input.major_version,
            input.full_version
        ],
    )?;
    ensure_found(changed, "kernel", id)?;
    get(conn, id)
}

/// Deletes a kernel. Profiles bound to it fall back to the default kernel (`ON DELETE SET NULL`).
pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    in_tx(conn, |conn| {
        let changed = conn.execute("DELETE FROM kernels WHERE id = ?1", [id])?;
        ensure_found(changed, "kernel", id)?;
        conn.execute(
            "UPDATE kernels SET is_default = 1
             WHERE id = (SELECT id FROM kernels ORDER BY created_at, rowid LIMIT 1)
               AND NOT EXISTS (SELECT 1 FROM kernels WHERE is_default = 1)",
            [],
        )?;
        Ok(())
    })
}

pub fn set_default(conn: &Connection, id: &str) -> AppResult<()> {
    // The EXISTS guard keeps an unknown id from clearing the current default.
    let changed = conn.execute(
        "UPDATE kernels SET is_default = (id = ?1)
         WHERE EXISTS (SELECT 1 FROM kernels WHERE id = ?1)",
        [id],
    )?;
    ensure_found(changed, "kernel", id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profiles;
    use crate::db::test_support::TestDb;
    use crate::error::ErrorCode;

    fn default_id(conn: &Connection) -> Option<String> {
        get_default(conn).unwrap().map(|k| k.id)
    }

    #[test]
    fn first_kernel_becomes_default_and_default_is_exclusive() {
        let db = TestDb::new();
        assert_eq!(get_default(&db.conn).unwrap(), None);

        let a = db.kernel("a", 142);
        let b = db.kernel("b", 144);
        assert!(a.is_default);
        assert!(!b.is_default);

        set_default(&db.conn, &b.id).unwrap();
        assert_eq!(default_id(&db.conn), Some(b.id.clone()));
        let defaults = list(&db.conn)
            .unwrap()
            .iter()
            .filter(|k| k.is_default)
            .count();
        assert_eq!(defaults, 1);

        let err = set_default(&db.conn, "missing").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(default_id(&db.conn), Some(b.id));
    }

    #[test]
    fn deleting_the_default_promotes_the_oldest_remaining() {
        let db = TestDb::new();
        let a = db.kernel("a", 142);
        let b = db.kernel("b", 144);
        let c = db.kernel("c", 150);
        set_default(&db.conn, &b.id).unwrap();

        delete(&db.conn, &b.id).unwrap();
        assert_eq!(default_id(&db.conn), Some(a.id.clone()));

        // Deleting a non-default kernel leaves the default alone.
        delete(&db.conn, &c.id).unwrap();
        assert_eq!(default_id(&db.conn), Some(a.id.clone()));

        delete(&db.conn, &a.id).unwrap();
        assert_eq!(default_id(&db.conn), None);
        assert_eq!(
            delete(&db.conn, &a.id).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn create_update_and_validation() {
        let db = TestDb::new();
        let path = db.abs_path("chrome");
        let kernel = create(
            &db.conn,
            KernelInput {
                name: "  Chromium 150 ".into(),
                exec_path: path.clone(),
                major_version: 150,
                full_version: Some(" ".into()),
            },
        )
        .unwrap();
        assert_eq!(kernel.name, "Chromium 150");
        assert_eq!(kernel.full_version, None);
        assert_eq!(list(&db.conn).unwrap(), std::slice::from_ref(&kernel));

        let updated = update(
            &db.conn,
            &kernel.id,
            KernelInput {
                name: "Chromium 144".into(),
                exec_path: path.clone(),
                major_version: 144,
                full_version: Some("144.0.7559.132".into()),
            },
        )
        .unwrap();
        assert_eq!(updated.major_version, 144);
        assert_eq!(updated.full_version.as_deref(), Some("144.0.7559.132"));
        assert!(updated.is_default);
        assert_eq!(get(&db.conn, &kernel.id).unwrap(), updated);

        let input = |exec_path: &str, major_version| KernelInput {
            name: "k".into(),
            exec_path: exec_path.into(),
            major_version,
            full_version: None,
        };
        let code = |r: AppResult<Kernel>| r.unwrap_err().code;
        assert_eq!(
            code(create(&db.conn, input(&path, 150))),
            ErrorCode::Conflict
        );
        assert_eq!(
            code(create(&db.conn, input("chrome", 150))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(&db.conn, input(&db.abs_path("x"), 0))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(update(&db.conn, "missing", input(&db.abs_path("y"), 150))),
            ErrorCode::NotFound
        );
        assert_eq!(code(get(&db.conn, "missing")), ErrorCode::NotFound);
    }

    #[test]
    fn deleting_a_kernel_unbinds_its_profiles() {
        let db = TestDb::new();
        let kernel = db.kernel("a", 144);
        let mut draft = TestDb::draft("p");
        draft.kernel_id = Some(kernel.id.clone());
        let profile = profiles::create(&db.conn, draft).unwrap();

        delete(&db.conn, &kernel.id).unwrap();
        let profile = profiles::get(&db.conn, &profile.id).unwrap();
        assert_eq!(profile.draft.kernel_id, None);
    }

    #[test]
    fn serializes_camel_case() {
        let db = TestDb::new();
        let kernel = db.kernel("a", 144);
        let json = serde_json::to_value(&kernel).unwrap();
        for key in [
            "id",
            "name",
            "execPath",
            "majorVersion",
            "fullVersion",
            "isDefault",
            "createdAt",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
    }
}

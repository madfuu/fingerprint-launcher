//! Error type shared by every layer. It reaches the frontend as `{ code, message }`
//! (design §5): `code` selects the i18n text, `message` carries the raw detail.

use std::fmt;

use rusqlite::ffi;
use serde::Serialize;

/// Stable error codes. The frontend keys its i18n messages on them, so renaming one is a
/// breaking change to the bridge contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The requested record does not exist.
    NotFound,
    /// A uniqueness constraint was violated, e.g. a group name or kernel path already in use.
    Conflict,
    /// The input failed validation or references a record that does not exist.
    InvalidInput,
    /// Unexpected SQLite failure, including a database that cannot be read or migrated.
    Db,
    /// Filesystem failure.
    Io,
}

impl ErrorCode {
    /// The wire form, identical to the serde serialization.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::InvalidInput => "invalid_input",
            Self::Db => "db",
            Self::Io => "io",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn db(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Db, message)
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        let code = match &err {
            rusqlite::Error::SqliteFailure(e, _)
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                match e.extended_code {
                    ffi::SQLITE_CONSTRAINT_UNIQUE | ffi::SQLITE_CONSTRAINT_PRIMARYKEY => {
                        ErrorCode::Conflict
                    }
                    // FOREIGNKEY, CHECK, NOTNULL: the caller sent a dangling id or a bad value.
                    _ => ErrorCode::InvalidInput,
                }
            }
            rusqlite::Error::QueryReturnedNoRows => ErrorCode::NotFound,
            _ => ErrorCode::Db,
        };
        Self::new(code, err.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_code_and_message() {
        let json = serde_json::to_value(AppError::invalid_input("bad seed")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "code": "invalid_input", "message": "bad seed" })
        );
    }

    #[test]
    fn code_as_str_matches_serde() {
        for code in [
            ErrorCode::NotFound,
            ErrorCode::Conflict,
            ErrorCode::InvalidInput,
            ErrorCode::Db,
            ErrorCode::Io,
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
        }
    }

    #[test]
    fn maps_sqlite_constraint_violations() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE parent (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE);
             CREATE TABLE child (parent_id TEXT REFERENCES parent(id), n INTEGER CHECK (n > 0));
             INSERT INTO parent VALUES ('a', 'x');",
        )
        .unwrap();
        let code = |sql: &str| AppError::from(conn.execute(sql, []).unwrap_err()).code;

        assert_eq!(
            code("INSERT INTO parent VALUES ('b', 'x')"),
            ErrorCode::Conflict
        );
        assert_eq!(
            code("INSERT INTO parent VALUES ('a', 'y')"),
            ErrorCode::Conflict
        );
        assert_eq!(
            code("INSERT INTO child VALUES ('missing', 1)"),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code("INSERT INTO child VALUES ('a', 0)"),
            ErrorCode::InvalidInput
        );
        assert_eq!(code("SELECT * FROM nowhere"), ErrorCode::Db);
    }
}

//! Proxy repository (design §2.2 `proxies`). Profiles reference a proxy entry instead of
//! storing their own copy (R6.1); deleting a proxy leaves its profiles direct
//! (`ON DELETE SET NULL`).
//!
//! There are deliberately no credential fields: fingerprint-chromium's `--proxy-server` does
//! not support proxy authentication (R6.2), so a username/password would silently not work.

use std::net::Ipv6Addr;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{clean_opt, col, ensure_found, new_id, not_found, now, required_text, select_list};
use crate::error::{AppError, AppResult};

text_enum! {
    /// Proxy protocol, as used in `--proxy-server=<scheme>://<host>:<port>`.
    pub enum ProxyScheme {
        Http => "http",
        Socks5 => "socks5",
    }
}

pub(crate) const COLUMNS: &[&str] = &[
    "id",
    "name",
    "scheme",
    "host",
    "port",
    "note",
    "created_at",
    "updated_at",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proxy {
    pub id: String,
    pub name: Option<String>,
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    pub note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Proxy {
    /// `<scheme>://<host>:<port>`, the `--proxy-server` value. An IPv6 host is bracketed
    /// (`socks5://[::1]:1080`); validation only lets a colon into the host as part of a bare
    /// IPv6 address.
    pub fn url(&self) -> String {
        if self.host.contains(':') {
            format!("{}://[{}]:{}", self.scheme, self.host, self.port)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }
}

/// Editable fields of a proxy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInput {
    #[serde(default)]
    pub name: Option<String>,
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub note: Option<String>,
}

impl ProxyInput {
    fn normalized(mut self) -> AppResult<Self> {
        self.host = required_text(&self.host, "proxy host")?;
        if self.host.contains('@') {
            return Err(AppError::invalid_input(
                "proxy credentials are not supported by fingerprint-chromium",
            ));
        }
        // A scheme, path or port pasted into the host field would produce a broken
        // --proxy-server. A colon is only valid as part of a bare IPv6 address.
        if self.host.contains(|c: char| c == '/' || c.is_whitespace())
            || (self.host.contains(':') && self.host.parse::<Ipv6Addr>().is_err())
        {
            return Err(AppError::invalid_input(format!(
                "proxy host must be a bare host name or IP address: {}",
                self.host
            )));
        }
        if self.port == 0 {
            return Err(AppError::invalid_input("proxy port must be 1-65535"));
        }
        clean_opt(&mut self.name);
        clean_opt(&mut self.note);
        Ok(self)
    }
}

/// Reads a proxy from columns selected with `select_list(COLUMNS, alias, prefix)`.
pub(crate) fn from_row(row: &Row<'_>, prefix: &str) -> rusqlite::Result<Proxy> {
    Ok(Proxy {
        id: col(row, prefix, "id")?,
        name: col(row, prefix, "name")?,
        scheme: col(row, prefix, "scheme")?,
        host: col(row, prefix, "host")?,
        port: col(row, prefix, "port")?,
        note: col(row, prefix, "note")?,
        created_at: col(row, prefix, "created_at")?,
        updated_at: col(row, prefix, "updated_at")?,
    })
}

fn select(where_order: &str) -> String {
    format!(
        "SELECT {} FROM proxies x {where_order}",
        select_list(COLUMNS, "x", "")
    )
}

/// All proxies, oldest first.
pub fn list(conn: &Connection) -> AppResult<Vec<Proxy>> {
    let mut stmt = conn.prepare(&select("ORDER BY x.created_at, x.rowid"))?;
    let rows = stmt.query_map([], |row| from_row(row, ""))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Proxy> {
    conn.query_row(&select("WHERE x.id = ?1"), [id], |row| from_row(row, ""))
        .optional()?
        .ok_or_else(|| not_found("proxy", id))
}

pub fn create(conn: &Connection, input: ProxyInput) -> AppResult<Proxy> {
    let input = input.normalized()?;
    let id = new_id();
    let now = now();
    conn.execute(
        "INSERT INTO proxies (id, name, scheme, host, port, note, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            id,
            input.name,
            input.scheme,
            input.host,
            input.port,
            input.note,
            now
        ],
    )?;
    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, input: ProxyInput) -> AppResult<Proxy> {
    let input = input.normalized()?;
    let changed = conn.execute(
        "UPDATE proxies SET name = ?2, scheme = ?3, host = ?4, port = ?5, note = ?6, updated_at = ?7
         WHERE id = ?1",
        params![
            id,
            input.name,
            input.scheme,
            input.host,
            input.port,
            input.note,
            now()
        ],
    )?;
    ensure_found(changed, "proxy", id)?;
    get(conn, id)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM proxies WHERE id = ?1", [id])?;
    ensure_found(changed, "proxy", id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profiles;
    use crate::db::test_support::TestDb;
    use crate::error::ErrorCode;

    fn input(scheme: ProxyScheme, host: &str, port: u16) -> ProxyInput {
        ProxyInput {
            name: None,
            scheme,
            host: host.into(),
            port,
            note: None,
        }
    }

    #[test]
    fn crud_round_trip() {
        let db = TestDb::new();
        let mut first = input(ProxyScheme::Socks5, " 127.0.0.1 ", 1080);
        first.name = Some(" Local ".into());
        first.note = Some("".into());
        let a = create(&db.conn, first).unwrap();
        assert_eq!(a.scheme, ProxyScheme::Socks5);
        assert_eq!(a.host, "127.0.0.1");
        assert_eq!(a.name.as_deref(), Some("Local"));
        assert_eq!(a.note, None);
        assert_eq!(a.created_at, a.updated_at);

        let b = create(
            &db.conn,
            input(ProxyScheme::Http, "proxy.example.com", 65535),
        )
        .unwrap();
        assert_eq!(list(&db.conn).unwrap(), [a.clone(), b.clone()]);

        let a2 = update(&db.conn, &a.id, input(ProxyScheme::Http, "10.0.0.1", 8080)).unwrap();
        assert_eq!(
            (a2.scheme, a2.host.as_str(), a2.port),
            (ProxyScheme::Http, "10.0.0.1", 8080)
        );
        assert_eq!(a2.created_at, a.created_at);
        assert!(a2.updated_at >= a.updated_at);

        delete(&db.conn, &a.id).unwrap();
        assert_eq!(list(&db.conn).unwrap(), [b]);
        assert_eq!(get(&db.conn, &a.id).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn rejects_invalid_host_port_and_credentials() {
        let db = TestDb::new();
        for (host, port) in [
            ("", 8080),
            ("user:pass@1.2.3.4", 8080),
            ("http://1.2.3.4", 8080),
            ("1.2.3.4 8080", 8080),
            ("1.2.3.4:8080", 8080),
            ("[::1]", 8080),
            ("1.2.3.4", 0),
        ] {
            let err = create(&db.conn, input(ProxyScheme::Http, host, port)).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{host}:{port}");
        }
        // A bare IPv6 address is a valid host; brackets are added when the URL is built.
        let v6 = db.proxy("::1", 1080);
        assert_eq!(v6.host, "::1");
        assert_eq!(v6.url(), "socks5://[::1]:1080");
        // Same host and port twice is allowed: entries may differ by name / note.
        db.proxy("1.2.3.4", 8080);
        db.proxy("1.2.3.4", 8080);
    }

    #[test]
    fn scheme_wire_format() {
        let parsed: ProxyInput = serde_json::from_value(serde_json::json!({
            "scheme": "socks5", "host": "h", "port": 1
        }))
        .unwrap();
        assert_eq!(parsed.scheme, ProxyScheme::Socks5);
        assert!(
            serde_json::from_value::<ProxyInput>(serde_json::json!({
                "scheme": "https", "host": "h", "port": 1
            }))
            .is_err()
        );
    }

    #[test]
    fn deleting_a_proxy_clears_profile_references() {
        let db = TestDb::new();
        let proxy = db.proxy("1.2.3.4", 8080);
        let mut draft = TestDb::draft("p");
        draft.proxy_id = Some(proxy.id.clone());
        let profile = profiles::create(&db.conn, draft).unwrap();

        delete(&db.conn, &proxy.id).unwrap();
        assert_eq!(
            profiles::get(&db.conn, &profile.id).unwrap().draft.proxy_id,
            None
        );
    }
}

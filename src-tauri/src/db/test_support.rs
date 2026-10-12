//! Test fixtures: a migrated database in a temporary directory plus record builders.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tempfile::TempDir;

use super::groups::{self, Group, GroupInput};
use super::kernels::{self, Kernel, KernelInput};
use super::profiles::{self, Profile, ProfileDraft};
use super::proxies::{self, Proxy, ProxyInput, ProxyScheme};
use super::tags::{self, Tag, TagInput};
use super::{DB_FILE_NAME, open};

pub(crate) struct TestDb {
    // Field order matters: fields drop in declaration order, and the connection must be closed
    // before the directory is removed (Windows cannot delete files that are still open).
    pub conn: Connection,
    dir: TempDir,
}

impl TestDb {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        let conn = open(&dir.path().join(DB_FILE_NAME)).expect("open database");
        Self { conn, dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().join(DB_FILE_NAME)
    }

    /// A second connection to the same file, as after an app restart. Drop it before `self`.
    pub fn reopen(&self) -> Connection {
        open(&self.path()).expect("reopen database")
    }

    /// An absolute path inside the temp dir, valid as a path setting on every platform.
    pub fn abs_path(&self, name: &str) -> String {
        path_string(&self.dir.path().join(name))
    }

    pub fn draft(name: &str) -> ProfileDraft {
        ProfileDraft {
            name: name.into(),
            ..ProfileDraft::default()
        }
    }

    pub fn profile(&self, name: &str) -> Profile {
        profiles::create(&self.conn, Self::draft(name)).expect("create profile")
    }

    pub fn group(&self, name: &str) -> Group {
        groups::create(
            &self.conn,
            GroupInput {
                name: name.into(),
                color: None,
                sort_order: 0,
            },
        )
        .expect("create group")
    }

    pub fn tag(&self, name: &str) -> Tag {
        tags::create(
            &self.conn,
            TagInput {
                name: name.into(),
                color: "#3b82f6".into(),
            },
        )
        .expect("create tag")
    }

    pub fn proxy(&self, host: &str, port: u16) -> Proxy {
        proxies::create(
            &self.conn,
            ProxyInput {
                name: None,
                scheme: ProxyScheme::Socks5,
                host: host.into(),
                port,
                note: None,
            },
        )
        .expect("create proxy")
    }

    pub fn kernel(&self, name: &str, major_version: u32) -> Kernel {
        kernels::create(
            &self.conn,
            KernelInput {
                name: name.into(),
                exec_path: self.abs_path(name),
                major_version,
                full_version: None,
            },
        )
        .expect("create kernel")
    }
}

fn path_string(path: &Path) -> String {
    path.to_str().expect("temp path is UTF-8").to_owned()
}

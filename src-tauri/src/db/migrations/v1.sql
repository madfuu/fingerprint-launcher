-- Schema v1 (design §2.2). Applied once by migrate.rs inside a transaction, which then
-- records meta.schema_version = 1. Never edit after release: add a v2 file instead.

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);  -- schema_version

CREATE TABLE kernels (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  exec_path     TEXT NOT NULL UNIQUE,
  major_version INTEGER NOT NULL,     -- 决定参数集（§3.2）
  full_version  TEXT,                 -- 仅展示，如 150.0.7871.186
  is_default    INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL
);

CREATE TABLE groups (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, color TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL
);

CREATE TABLE tags (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, color TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE proxies (
  id         TEXT PRIMARY KEY,
  name       TEXT,
  scheme     TEXT NOT NULL CHECK (scheme IN ('http','socks5')),
  host       TEXT NOT NULL,
  port       INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
  note       TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE profiles (
  id        TEXT PRIMARY KEY,
  name      TEXT NOT NULL,
  note      TEXT,
  group_id  TEXT REFERENCES groups(id)  ON DELETE SET NULL,
  kernel_id TEXT REFERENCES kernels(id) ON DELETE SET NULL,  -- NULL = 用默认内核
  proxy_id  TEXT REFERENCES proxies(id) ON DELETE SET NULL,

  -- 指纹（NULL = 不传该参数）
  fingerprint_seed        INTEGER,
  fp_platform             TEXT CHECK (fp_platform IN ('windows','linux','macos')),
  fp_platform_version     TEXT,
  fp_brand                TEXT,
  fp_brand_version        TEXT,
  fp_hardware_concurrency INTEGER,

  -- 版本相关（§3.2 门控）
  fp_gpu_vendor           TEXT,     -- 139–143
  fp_gpu_renderer         TEXT,     -- 139–143
  disable_gpu_fingerprint INTEGER NOT NULL DEFAULT 0,  -- 142–143
  disable_spoofing        TEXT,     -- 144+，CSV

  disable_non_proxied_udp INTEGER NOT NULL DEFAULT 1,
  lang          TEXT,
  accept_lang   TEXT,
  timezone      TEXT,
  headless      INTEGER NOT NULL DEFAULT 0,

  user_data_dir TEXT,               -- NULL = <user_data_root>/<id>
  start_urls    TEXT,               -- JSON 数组
  extra_args    TEXT,               -- 原样追加

  last_launched_at TEXT,
  deleted_at       TEXT,            -- 软删除（回收站）
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
CREATE INDEX idx_profiles_group   ON profiles(group_id);
CREATE INDEX idx_profiles_deleted ON profiles(deleted_at);

CREATE TABLE profile_tags (
  profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  tag_id     TEXT NOT NULL REFERENCES tags(id)     ON DELETE CASCADE,
  PRIMARY KEY (profile_id, tag_id)
);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

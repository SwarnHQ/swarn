-- Schema v1, copied from the technical PRD §3.1. Bump PRAGMA user_version for every change.

CREATE TABLE machine (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  schema_min    INTEGER NOT NULL,
  schema_max    INTEGER NOT NULL,
  created_at    TEXT NOT NULL
);

CREATE TABLE repo (
  id            TEXT PRIMARY KEY,
  path          TEXT NOT NULL UNIQUE,
  default_branch TEXT NOT NULL,
  remote_url    TEXT,
  setup_script  TEXT,
  created_at    TEXT NOT NULL
);

CREATE TABLE workspace (
  id            TEXT PRIMARY KEY,
  repo_id       TEXT NOT NULL REFERENCES repo(id),
  name          TEXT NOT NULL,
  branch        TEXT NOT NULL,
  base_ref      TEXT NOT NULL,
  worktree_path TEXT NOT NULL,
  state         TEXT NOT NULL,     -- creating|ready|archived|failed
  created_at    TEXT NOT NULL,
  archived_at   TEXT
);

CREATE TABLE session (
  id            TEXT PRIMARY KEY,
  workspace_id  TEXT NOT NULL REFERENCES workspace(id),
  agent         TEXT NOT NULL,     -- claude-code|codex|custom
  tier          TEXT NOT NULL,     -- native|acp|pty (acp and pty from v0.2)
  status        TEXT NOT NULL,     -- starting|working|needs_input|done|failed|exited
  pid           INTEGER,
  acp_session   TEXT,              -- agent-side session or thread id for resume
  started_at    TEXT NOT NULL,
  ended_at      TEXT,
  last_change_at TEXT NOT NULL
);

CREATE TABLE event (
  seq           INTEGER PRIMARY KEY AUTOINCREMENT,  -- monotonic per daemon
  session_id    TEXT NOT NULL REFERENCES session(id),
  kind          TEXT NOT NULL,
  payload       TEXT NOT NULL,     -- JSON
  ts            TEXT NOT NULL
);
CREATE INDEX event_session_seq ON event(session_id, seq);

CREATE TABLE approval (
  request_id    TEXT PRIMARY KEY,
  session_id    TEXT NOT NULL REFERENCES session(id),
  tool          TEXT NOT NULL,
  risk          TEXT NOT NULL,
  nonce         BLOB NOT NULL,
  expires_at    TEXT NOT NULL,
  resolved_at   TEXT,
  decision      TEXT,
  device_id     TEXT,
  sig           BLOB
);

CREATE TABLE device (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  platform      TEXT NOT NULL,
  x25519_pub    BLOB NOT NULL,
  p256_pub      BLOB NOT NULL,
  push_token    TEXT,
  capability    TEXT,              -- gateway token
  capability_exp TEXT,
  paired_at     TEXT NOT NULL,
  revoked_at    TEXT
);

CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);

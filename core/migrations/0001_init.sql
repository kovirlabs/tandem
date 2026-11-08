-- Tandem initial schema (SPEC §6.1). Images are never stored.
CREATE TABLE tasks (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL,
  notes      TEXT,
  created_at TEXT NOT NULL,
  done_at    TEXT
);

CREATE TABLE subtasks (
  id       TEXT PRIMARY KEY,
  task_id  TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  title    TEXT NOT NULL,
  position INTEGER NOT NULL,
  done_at  TEXT
);

CREATE TABLE sessions (
  id             TEXT PRIMARY KEY,
  task_id        TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  planned_min    INTEGER NOT NULL,
  started_at     TEXT NOT NULL,
  ended_at       TEXT,
  state          TEXT NOT NULL,
  checkin_mode   TEXT NOT NULL,
  camera_enabled INTEGER NOT NULL DEFAULT 0,
  cloud_vision   INTEGER NOT NULL DEFAULT 0,
  reflection     TEXT
);

-- kinds: start, pause, resume, extend, checkin, nudge, away, back, end
CREATE TABLE session_events (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  ts         TEXT NOT NULL,
  kind       TEXT NOT NULL,
  payload    TEXT NOT NULL DEFAULT '{}'
);

-- via: voice | text | checkin. Retention is configurable (default 30 days).
CREATE TABLE messages (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  ts         TEXT NOT NULL,
  role       TEXT NOT NULL,
  content    TEXT NOT NULL,
  via        TEXT NOT NULL
);

CREATE TABLE profile  (key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- Non-secret settings only. API keys live in the OS keychain.
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE INDEX idx_sessions_started ON sessions(started_at);
CREATE INDEX idx_messages_ts ON messages(ts);
CREATE INDEX idx_events_session ON session_events(session_id);

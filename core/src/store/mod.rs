//! SQLite repository. Small synchronous operations behind a mutex; never stores images or secrets.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use ts_rs::TS;

use crate::error::{CoreError, Result};

const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_init.sql")];

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct HistoryEntry {
    pub session_id: String,
    pub task: String,
    pub planned_min: u32,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub reflection: Option<String>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let target = i as i64 + 1;
            if version < target {
                conn.execute_batch(&format!(
                    "BEGIN; {sql}\nPRAGMA user_version = {target}; COMMIT;"
                ))?;
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn create_task(&self, title: &str) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO tasks (id, title, created_at) VALUES (?1, ?2, ?3)",
            params![id, title, now()],
        )?;
        Ok(id)
    }

    pub fn add_subtask(&self, task_id: &str, title: &str, position: i64) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO subtasks (id, task_id, title, position) VALUES (?1, ?2, ?3, ?4)",
            params![id, task_id, title, position],
        )?;
        Ok(id)
    }

    pub fn complete_subtask(&self, id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE subtasks SET done_at = ?2 WHERE id = ?1 AND done_at IS NULL",
            params![id, now()],
        )?;
        Ok(())
    }

    pub fn create_session(
        &self,
        task_id: &str,
        planned_min: u32,
        checkin_mode: &str,
        camera_enabled: bool,
        cloud_vision: bool,
    ) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO sessions (id, task_id, planned_min, started_at, state, checkin_mode, camera_enabled, cloud_vision)
             VALUES (?1, ?2, ?3, ?4, 'focusing', ?5, ?6, ?7)",
            params![id, task_id, planned_min, now(), checkin_mode, camera_enabled, cloud_vision],
        )?;
        Ok(id)
    }

    pub fn set_session_state(&self, id: &str, state: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE sessions SET state = ?2 WHERE id = ?1",
            params![id, state],
        )?;
        Ok(())
    }

    /// Mark ended (first call wins for `ended_at`); a later call can add the reflection.
    pub fn end_session(&self, id: &str, reflection: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE sessions SET state = 'ended', ended_at = COALESCE(ended_at, ?2),
                 reflection = COALESCE(?3, reflection) WHERE id = ?1",
            params![id, now(), reflection],
        )?;
        Ok(())
    }

    pub fn add_event(
        &self,
        session_id: &str,
        kind: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO session_events (session_id, ts, kind, payload) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, now(), kind, payload.to_string()],
        )?;
        Ok(())
    }

    pub fn add_message(
        &self,
        session_id: Option<&str>,
        role: &str,
        content: &str,
        via: &str,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO messages (session_id, ts, role, content, via) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, now(), role, content, via],
        )?;
        Ok(())
    }

    /// Delete messages older than `days`. Returns how many were removed.
    pub fn purge_messages_older_than(&self, days: u32) -> Result<usize> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(i64::from(days))).to_rfc3339();
        Ok(self
            .conn()
            .execute("DELETE FROM messages WHERE ts < ?1", params![cutoff])?)
    }

    pub fn recent_sessions(&self, limit: u32) -> Result<Vec<HistoryEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT s.id, t.title, s.planned_min, s.started_at, s.ended_at, s.reflection
             FROM sessions s JOIN tasks t ON t.id = s.task_id
             ORDER BY s.started_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(HistoryEntry {
                session_id: r.get(0)?,
                task: r.get(1)?,
                planned_min: r.get(2)?,
                started_at: r.get(3)?,
                ended_at: r.get(4)?,
                reflection: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn get_json<T: DeserializeOwned>(&self, table: &str, key: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .conn()
            .query_row(
                &format!("SELECT value FROM {table} WHERE key = ?1"),
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        match raw {
            Some(s) => serde_json::from_str(&s)
                .map(Some)
                .map_err(|e| CoreError::Invalid(format!("{table}.{key}: {e}"))),
            None => Ok(None),
        }
    }

    fn set_json<T: Serialize>(&self, table: &str, key: &str, value: &T) -> Result<()> {
        let s = serde_json::to_string(value).map_err(|e| CoreError::Invalid(e.to_string()))?;
        self.conn().execute(
            &format!("INSERT INTO {table} (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value"),
            params![key, s],
        )?;
        Ok(())
    }

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.get_json("settings", key)
    }
    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.set_json("settings", key, value)
    }
    pub fn get_profile<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.get_json("profile", key)
    }
    pub fn set_profile<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.set_json("profile", key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent_on_disk() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("t.db");
        {
            let s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
            assert!(s.create_task("x").is_ok());
        }
        let s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(s.create_task("y").is_ok());
    }

    #[test]
    fn session_lifecycle_and_history() -> Result<()> {
        let s = Store::open_in_memory()?;
        let t = s.create_task("expense report")?;
        let sub = s.add_subtask(&t, "find receipts", 0)?;
        s.complete_subtask(&sub)?;
        let id = s.create_session(&t, 25, "gentle", false, false)?;
        s.add_event(&id, "start", &serde_json::json!({"minutes": 25}))?;
        s.set_session_state(&id, "paused")?;
        s.end_session(&id, None)?;
        s.end_session(&id, Some("filed 3"))?;
        s.add_message(Some(&id), "user", "hi", "text")?;
        s.add_message(None, "assistant", "hello", "text")?;
        let h = s.recent_sessions(5)?;
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].task, "expense report");
        assert_eq!(h[0].reflection.as_deref(), Some("filed 3"));
        assert!(h[0].ended_at.is_some());
        assert_eq!(s.purge_messages_older_than(30)?, 0);
        assert_eq!(s.purge_messages_older_than(0)?, 2);
        Ok(())
    }

    #[test]
    fn json_key_values() -> Result<()> {
        let s = Store::open_in_memory()?;
        assert_eq!(s.get_setting::<String>("k")?, None);
        s.set_setting("k", &"v")?;
        s.set_setting("k", &"w")?;
        assert_eq!(s.get_setting::<String>("k")?, Some("w".into()));
        s.set_profile("about", &vec![1, 2])?;
        assert_eq!(s.get_profile::<Vec<i32>>("about")?, Some(vec![1, 2]));
        assert!(s.get_profile::<bool>("about").is_err());
        Ok(())
    }
}

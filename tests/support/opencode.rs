use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct Fixture {
    pub conn: Connection,
    pub path: PathBuf,
    _dir: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
            CREATE TABLE session_v2 (
                id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT,
                time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
                time_archived INTEGER, version TEXT,
                tokens_input INTEGER NOT NULL DEFAULT 0,
                tokens_output INTEGER NOT NULL DEFAULT 0,
                tokens_reasoning INTEGER NOT NULL DEFAULT 0,
                tokens_cache_read INTEGER NOT NULL DEFAULT 0,
                tokens_cache_write INTEGER NOT NULL DEFAULT 0,
                title TEXT, summary_diffs TEXT
            );
            CREATE TABLE session_message (
                id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
                time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            CREATE INDEX session_message_time_created_idx ON session_message(time_created);
            CREATE INDEX session_message_session_type_idx ON session_message(session_id, type);
            CREATE TABLE migration (id TEXT PRIMARY KEY);
            INSERT INTO migration VALUES ('20261116213417_automation_active_index');",
        )
        .unwrap();
        Self {
            conn,
            path,
            _dir: dir,
        }
    }

    pub fn session(&self, id: &str, parent: Option<&str>, created: i64, updated: i64) {
        self.conn
            .execute(
                "INSERT INTO session_v2
             (id,parent_id,directory,time_created,time_updated,version,title,summary_diffs)
             VALUES (?1,?2,?3,?4,?5,'2.3.2','fixture title','fixture diff')",
                params![
                    id,
                    parent,
                    format!("/private/projects/{id}"),
                    at(created),
                    at(updated)
                ],
            )
            .unwrap();
    }

    pub fn message(&self, id: &str, session: &str, created: i64, updated: i64, data: Value) {
        self.conn
            .execute(
                "INSERT INTO session_message (id,session_id,type,time_created,time_updated,data)
             VALUES (?1,?2,'assistant',?3,?4,?5)
             ON CONFLICT (id) DO UPDATE SET time_updated=excluded.time_updated, data=excluded.data",
                params![id, session, at(created), at(updated), data.to_string()],
            )
            .unwrap();
    }
}

pub fn time(minute: i64) -> DateTime<Utc> {
    let now: DateTime<Utc> = "2026-07-17T12:00:00Z".parse().unwrap();
    now + Duration::minutes(minute)
}

pub fn at(minute: i64) -> i64 {
    time(minute).timestamp_millis()
}

pub fn usage(model: &str, upstream: &str, completed: i64, input: u64, output: u64) -> Value {
    json!({
        "model": {"id": model, "providerID": upstream},
        "time": {"completed": at(completed)},
        "finish": "stop",
        "tokens": {"input": input, "output": output, "reasoning": 0, "cache": {"read": 0, "write": 0}},
        "content": "fixture conversation"
    })
}

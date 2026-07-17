//! Reader for the Codex CLI state database (`~/.codex/state_N.sqlite`).
//!
//! Newer Codex versions (observed with 0.144.x) run an in-process
//! app-server and record sessions as rows in a `threads` table instead of
//! (or in addition to) rollout JSONL files. Each row carries `id`, `cwd`,
//! `model`, a cumulative `tokens_used` total, and second-resolution
//! `created_at` / `updated_at` stamps — enough for the session table and
//! observed-token deltas, though not for an input/output split (those
//! tokens land in `TokenCounts::unattributed`) and not for rate limits
//! (use `--live` for quota when rollout files are absent).
//!
//! The database is opened read-only and every failure degrades to `None`:
//! a monitoring tool must never disturb — or be killed by — the CLI that
//! owns the file.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ThreadRow {
    pub id: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub tokens_used: u64,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Locate the newest `state_N.sqlite` under a Codex home. The CLI bumps
/// the numeric suffix on schema migrations, so the highest N wins.
pub fn find_state_db(codex_home: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(codex_home).ok()?.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(n) = name
            .strip_prefix("state_")
            .and_then(|rest| rest.strip_suffix(".sqlite"))
            .and_then(|n| n.parse::<u64>().ok())
        else {
            continue;
        };
        if best.as_ref().is_none_or(|(b, _)| n > *b) {
            best = Some((n, path));
        }
    }
    best.map(|(_, p)| p)
}

/// Threads updated at or after `since`, newest first. Any error (locked,
/// missing table, schema drift) yields `None` rather than failing the scan.
pub fn threads_since(db_path: &Path, since: DateTime<Utc>) -> Option<Vec<ThreadRow>> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    // Never wait on the CLI's write locks.
    conn.busy_timeout(std::time::Duration::from_millis(100))
        .ok()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, cwd, model, tokens_used, created_at, updated_at \
             FROM threads WHERE updated_at >= ?1 AND archived = 0 \
             ORDER BY updated_at DESC",
        )
        .ok()?;
    let rows = stmt
        .query_map([since.timestamp()], |row| {
            Ok(ThreadRow {
                id: row.get::<_, String>(0)?,
                cwd: row.get::<_, Option<String>>(1)?,
                model: row.get::<_, Option<String>>(2)?,
                tokens_used: row.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) as u64,
                created_at: row
                    .get::<_, Option<i64>>(4)?
                    .and_then(|s| Utc.timestamp_opt(s, 0).single()),
                updated_at: row
                    .get::<_, Option<i64>>(5)?
                    .and_then(|s| Utc.timestamp_opt(s, 0).single()),
            })
        })
        .ok()?
        .filter_map(Result::ok)
        .collect();
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                cwd TEXT,
                model TEXT,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO threads VALUES
              ('t1', '/home/u/projects/app', 'gpt-5.6-terra', 644049, 1784000000, 1784003600, 0),
              ('t2', '/home/u/projects/old', 'gpt-5.5', 999, 1700000000, 1700000100, 0),
              ('t3', '/home/u/projects/arch', 'gpt-5.5', 5, 1784000000, 1784003600, 1);",
        )
        .unwrap();
        path
    }

    #[test]
    fn reads_recent_unarchived_threads() {
        let dir = std::env::temp_dir().join(format!("lmtop-codexdb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = make_db(&dir, "state_5.sqlite");
        let since = Utc.timestamp_opt(1784000000, 0).single().unwrap();
        let rows = threads_since(&path, since).unwrap();
        assert_eq!(rows.len(), 1); // t2 too old, t3 archived
        assert_eq!(rows[0].id, "t1");
        assert_eq!(rows[0].tokens_used, 644049);
        assert_eq!(rows[0].model.as_deref(), Some("gpt-5.6-terra"));
        assert_eq!(rows[0].cwd.as_deref(), Some("/home/u/projects/app"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn picks_highest_numbered_state_db() {
        let dir = std::env::temp_dir().join(format!("lmtop-codexdb-pick-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        make_db(&dir, "state_5.sqlite");
        let newest = make_db(&dir, "state_12.sqlite");
        std::fs::write(dir.join("state_x.sqlite"), b"junk").unwrap();
        assert_eq!(find_state_db(&dir), Some(newest));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_db_yields_none() {
        let dir = std::env::temp_dir().join(format!("lmtop-codexdb-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state_1.sqlite");
        std::fs::write(&path, b"this is not a sqlite database").unwrap();
        assert!(threads_since(&path, Utc::now()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Read-only OpenCode SQLite usage metadata. SQL extracts only the fields
//! needed for accounting; never return the message JSON itself.

use super::store::{UsageStore, project_name};
use super::{Collector, ScanContext};
use crate::config::OpenCodeConfig;
use crate::domain::{
    Capability, CollectorHealth, CollectorStatus, ModelIdentity, Provider, ProviderSnapshot,
    TokenCounts,
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, Row};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const CAPABILITIES: [Capability; 5] = [
    Capability::LocalTokenUsage,
    Capability::ActiveSession,
    Capability::CalendarWeekAggregation,
    Capability::ModelBreakdown,
    Capability::History,
];
const BUSY_TIMEOUT: Duration = Duration::from_millis(100);

// Keep the token paths shared with reconciliation. Do not project the JSON
// object, even to discover new token categories.
macro_rules! message_token_columns {
    () => {
        r#"json_extract(m.data, '$.tokens.input') AS input,
           json_extract(m.data, '$.tokens.output') AS output,
           json_extract(m.data, '$.tokens.reasoning') AS reasoning,
           json_extract(m.data, '$.tokens.cache.read') AS cache_read,
           json_extract(m.data, '$.tokens.cache.write') AS cache_write"#
    };
}

const MESSAGES: &str = concat!(
    r#"SELECT m.id, m.session_id, m.time_created, m.time_updated,
              json_extract(m.data, '$.model.providerID'),
              json_extract(m.data, '$.model.id'),
              json_extract(m.data, '$.model.variant'),
              json_extract(m.data, '$.time.completed'),
              json_extract(m.data, '$.finish'), "#,
    message_token_columns!(),
    " FROM session_message m WHERE m.type = 'assistant' AND m.time_created >= ?1",
);

const SESSION: &str = "SELECT parent_id, directory, time_created, time_updated \
                       FROM session_v2 WHERE id = ?1";

// Session counters are lifetime counters. Subtract lifetime message usage,
// not the retention-limited scan, and check *all* assistant upstreams,
// including incomplete messages. No positive inference from an unknown
// upstream or a session with no assistant usage to reconcile.
const RESIDUALS: &str = concat!(
    r#"WITH messages AS (
        SELECT m.session_id,
               json_extract(m.data, '$.model.providerID') AS upstream,
               json_extract(m.data, '$.time.completed') AS completed, "#,
    message_token_columns!(),
    r#" FROM session_v2 s CROSS JOIN session_message m ON m.session_id = s.id
        WHERE s.time_updated >= ?1 AND m.type = 'assistant'
    ), totals AS (
        SELECT session_id,
               sum(input) FILTER (WHERE completed IS NOT NULL) AS input,
               sum(output) FILTER (WHERE completed IS NOT NULL) AS output,
               sum(reasoning) FILTER (WHERE completed IS NOT NULL) AS reasoning,
               sum(cache_read) FILTER (WHERE completed IS NOT NULL) AS cache_read,
               sum(cache_write) FILTER (WHERE completed IS NOT NULL) AS cache_write
        FROM messages
        GROUP BY session_id
        HAVING count(*) FILTER (WHERE upstream IS NULL OR upstream = ''
                    OR upstream IN (SELECT value FROM json_each(?2))) = 0
           AND count(*) FILTER (WHERE completed IS NOT NULL AND coalesce(
                     input, output, reasoning, cache_read, cache_write) IS NOT NULL) > 0
    )
    SELECT s.id, s.time_updated,
           max(0, s.tokens_input - coalesce(t.input, 0)),
           max(0, s.tokens_output - coalesce(t.output, 0)),
           max(0, s.tokens_reasoning - coalesce(t.reasoning, 0)),
           max(0, s.tokens_cache_read - coalesce(t.cache_read, 0)),
           max(0, s.tokens_cache_write - coalesce(t.cache_write, 0))
    FROM totals t JOIN session_v2 s ON s.id = t.session_id
    ORDER BY s.id"#,
);

const SESSION_COLUMNS: &[&str] = &[
    "id",
    "parent_id",
    "directory",
    "time_created",
    "time_updated",
    "time_archived",
    "version",
    "tokens_input",
    "tokens_output",
    "tokens_reasoning",
    "tokens_cache_read",
    "tokens_cache_write",
];
const MESSAGE_COLUMNS: &[&str] = &[
    "id",
    "session_id",
    "type",
    "time_created",
    "time_updated",
    "data",
];

struct Message {
    id: String,
    session_id: String,
    created: DateTime<Utc>,
    updated: DateTime<Utc>,
    completed: Option<DateTime<Utc>>,
    upstream: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    failed: bool,
    tokens: Option<TokenCounts>,
}

struct LedgerEntry {
    created: DateTime<Utc>,
    reported: TokenCounts,
    counted: TokenCounts,
}

#[derive(PartialEq, Eq)]
struct Residual {
    session_id: String,
    at: DateTime<Utc>,
    tokens: TokenCounts,
}

struct SessionRow {
    parent: Option<String>,
    project: Option<String>,
    created: DateTime<Utc>,
    updated: DateTime<Utc>,
}

enum ReadError {
    Unsupported(Option<String>),
    Sql(rusqlite::Error),
    InvalidUsage,
}

impl From<rusqlite::Error> for ReadError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sql(err)
    }
}

pub struct OpenCodeCollector {
    enabled: bool,
    paths: Vec<PathBuf>,
    skip_upstreams: Vec<String>,
    high_water: Option<DateTime<Utc>>,
    ledger: HashMap<String, LedgerEntry>,
    residuals: Vec<Residual>,
    store: UsageStore,
    failed_messages: usize,
}

impl OpenCodeCollector {
    pub fn from_config(cfg: &OpenCodeConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            paths: database_candidates(cfg.db_path.as_deref()),
            skip_upstreams: cfg.skip_upstreams.clone(),
            high_water: None,
            ledger: HashMap::new(),
            residuals: Vec::new(),
            store: UsageStore::new(Provider::OpenCode),
            failed_messages: 0,
        }
    }

    fn scan_db(&mut self, path: &Path, ctx: &ScanContext) -> Result<(), ReadError> {
        let mut db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(BUSY_TIMEOUT)?;
        // Read messages and session counters from the same SQLite snapshot.
        let db = db.transaction()?;
        if !has_columns(&db, "PRAGMA table_info(session_v2)", SESSION_COLUMNS)?
            || !has_columns(&db, "PRAGMA table_info(session_message)", MESSAGE_COLUMNS)?
        {
            return Err(ReadError::Unsupported(latest_migration(&db)?));
        }

        let cutoff = ctx.retention_cutoff();
        let mut stmt = db.prepare(MESSAGES)?;
        let messages = stmt
            .query_map([cutoff.timestamp_millis()], read_message)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let high_water = messages.iter().map(|m| m.updated).max();
        let skip = serde_json::json!(self.skip_upstreams).to_string();
        let mut stmt = db.prepare(RESIDUALS)?;
        let residuals = stmt
            .query_map(rusqlite::params![cutoff.timestamp_millis(), skip], |row| {
                Ok(Residual {
                    session_id: row.get(0)?,
                    at: date(row, 1)?,
                    tokens: read_tokens(row, 2)?.ok_or(rusqlite::Error::InvalidQuery)?,
                })
            })?
            .filter(|r| !r.as_ref().is_ok_and(|r| r.tokens.is_zero()))
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Rebuild only on changes or when entries age out. Replacing the
        // derived store lets provisional residuals shrink as messages arrive.
        if self.high_water.is_some()
            && high_water == self.high_water
            && self.residuals == residuals
            && !self.ledger.values().any(|entry| entry.created < cutoff)
        {
            return Ok(());
        }
        self.rebuild(&db, ctx, messages, residuals)?;
        self.high_water = high_water;
        Ok(())
    }

    fn rebuild(
        &mut self,
        db: &Connection,
        ctx: &ScanContext,
        mut messages: Vec<Message>,
        residuals: Vec<Residual>,
    ) -> Result<(), ReadError> {
        let mut store = UsageStore::new(Provider::OpenCode);
        let mut total = TokenCounts::default();
        let mut sessions = Sessions::default();
        let mut ledger = HashMap::new();
        let mut failed_messages = 0;

        for residual in &residuals {
            let root = sessions.root(db, &residual.session_id)?;
            check_sum(&mut total, &residual.tokens)?;
            store.record_event(&root, residual.at, None, residual.tokens.clone());
            let row = sessions.rows.get(&root).ok_or(ReadError::InvalidUsage)?;
            let record = store.session_mut(&root);
            record.started_at = Some(row.created);
            record.last_activity = Some(row.updated);
            record.project.clone_from(&row.project);
        }
        // Apply metadata from the latest counted message, not the most
        // recently updated row or the session's auxiliary event.
        messages.sort_unstable_by(|a, b| (a.completed, &a.id).cmp(&(b.completed, &b.id)));
        for m in messages {
            let Some(at) = m.completed else { continue };
            if m.failed && m.tokens.as_ref().is_none_or(TokenCounts::is_zero) {
                failed_messages += 1;
                continue;
            }
            let Some(reported) = m.tokens else { continue };
            let upstream = m
                .upstream
                .filter(|s| !s.is_empty())
                .ok_or(ReadError::InvalidUsage)?;
            if self.skip_upstreams.contains(&upstream) {
                continue;
            }
            let model = m
                .model
                .filter(|s| !s.is_empty())
                .ok_or(ReadError::InvalidUsage)?;
            let root = sessions.root(db, &m.session_id)?;
            let counted = if let Some(old) = self.ledger.get(&m.id) {
                let mut counted = old.counted.clone();
                check_sum(
                    &mut counted,
                    &TokenCounts::saturating_delta(&reported, &old.reported),
                )?;
                counted
            } else {
                reported.clone()
            };
            check_sum(&mut total, &counted)?;
            store.record_event(
                &root,
                at,
                Some(ModelIdentity::normalize(&model)),
                counted.clone(),
            );
            let row = sessions.rows.get(&root).ok_or(ReadError::InvalidUsage)?;
            let record = store.session_mut(&root);
            record.started_at = Some(row.created);
            record.last_activity = Some(at);
            record.project.clone_from(&row.project);
            record.reasoning_effort = m.effort;
            record.context_tokens = Some(reported.total_input());
            ledger.insert(
                m.id,
                LedgerEntry {
                    created: m.created,
                    reported,
                    counted,
                },
            );
        }

        store.trim(ctx);
        self.store = store;
        self.ledger = ledger;
        self.residuals = residuals;
        self.failed_messages = failed_messages;
        Ok(())
    }
}

impl Collector for OpenCodeCollector {
    fn provider(&self) -> Provider {
        Provider::OpenCode
    }

    fn scan(&mut self, ctx: &ScanContext) -> ProviderSnapshot {
        let result = if !self.enabled {
            Err(CollectorHealth::disabled())
        } else {
            let path = self
                .paths
                .iter()
                .find(|p| p.try_exists().unwrap_or(true))
                .cloned();
            match path {
                Some(path) => self.scan_db(&path, ctx).map_err(read_health),
                None => Err(CollectorHealth::unavailable("no OpenCode database found")),
            }
        };
        match result {
            Ok(()) => self.store.build_snapshot(
                ctx,
                CAPABILITIES.to_vec(),
                CollectorHealth {
                    status: CollectorStatus::Ok,
                    message: (self.failed_messages > 0)
                        .then(|| format!("{} failed messages without usage", self.failed_messages)),
                    last_scan: Some(ctx.now),
                    files_scanned: 1,
                    parse_errors: 0,
                },
                vec![],
                None,
            ),
            Err(health) => ProviderSnapshot::empty(self.provider(), health),
        }
    }
}

fn database_candidates(configured: Option<&Path>) -> Vec<PathBuf> {
    let home = directories::BaseDirs::new().map(|base| base.home_dir().to_owned());
    if let Some(path) = configured {
        if let (Some(home), Ok(relative)) = (&home, path.strip_prefix("~")) {
            return vec![home.join(relative)];
        }
        return vec![path.to_owned()];
    }
    let mut paths = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        paths.push(xdg.join("opencode/opencode.db"));
    }
    if let Some(home) = home {
        paths.push(home.join(".local/share/opencode/opencode.db"));
        #[cfg(target_os = "macos")]
        paths.push(home.join("Library/Application Support/opencode/opencode.db"));
    }
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join("opencode/opencode.db"));
    }
    paths
}

fn read_health(error: ReadError) -> CollectorHealth {
    if let ReadError::Unsupported(migration) = error {
        return CollectorHealth::unavailable(format!(
            "unsupported OpenCode schema (latest migration {})",
            migration.as_deref().unwrap_or("unknown")
        ));
    }
    // Do not report raw SQLite errors: they can include paths or row data.
    let message = match error {
        ReadError::Sql(err)
            if matches!(
                err.sqlite_error_code(),
                Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
            ) =>
        {
            "OpenCode database is locked"
        }
        ReadError::Sql(_) => "could not read OpenCode database",
        _ => "invalid OpenCode usage metadata",
    };
    CollectorHealth {
        status: CollectorStatus::Degraded,
        parse_errors: 1,
        ..CollectorHealth::unavailable(message)
    }
}

fn has_columns(db: &Connection, pragma: &str, required: &[&str]) -> rusqlite::Result<bool> {
    let mut stmt = db.prepare(pragma)?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(required.iter().all(|name| names.contains(*name)))
}

fn latest_migration(db: &Connection) -> rusqlite::Result<Option<String>> {
    if !has_columns(db, "PRAGMA table_info(migration)", &["id"])? {
        return Ok(None);
    }
    let id: Option<String> = db
        .query_row(
            "SELECT id FROM migration ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    // Migration IDs are identifiers, not arbitrary text for display.
    Ok(id
        .filter(|id| id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')))
}

fn read_message(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        session_id: row.get(1)?,
        created: date(row, 2)?,
        updated: date(row, 3)?,
        upstream: row.get(4)?,
        model: row.get(5)?,
        effort: row
            .get_ref(6)?
            .as_str()
            .ok()
            .filter(|s| matches!(*s, "minimal" | "low" | "medium" | "high" | "xhigh"))
            .map(str::to_owned),
        completed: row.get::<_, Option<i64>>(7)?.map(timestamp).transpose()?,
        failed: row.get::<_, Option<String>>(8)?.as_deref() == Some("error"),
        tokens: read_tokens(row, 9)?,
    })
}

fn timestamp(ms: i64) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms).ok_or(rusqlite::Error::IntegralValueOutOfRange(0, ms))
}

fn date(row: &Row<'_>, col: usize) -> rusqlite::Result<DateTime<Utc>> {
    timestamp(row.get(col)?)
}

fn read_tokens(row: &Row<'_>, start: usize) -> rusqlite::Result<Option<TokenCounts>> {
    let [input, output, reasoning, cached_input, cache_creation] = [0, 1, 2, 3, 4].map(|offset| {
        let col = start + offset;
        row.get::<_, Option<i64>>(col)?
            .map(|n| u64::try_from(n).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(col, n)))
            .transpose()
    });
    let (input, output, reasoning, cached_input, cache_creation) =
        (input?, output?, reasoning?, cached_input?, cache_creation?);
    if [input, output, reasoning, cached_input, cache_creation]
        .iter()
        .all(Option::is_none)
    {
        return Ok(None);
    }
    let reasoning = reasoning.unwrap_or_default();
    // SQLite integers are at most i64::MAX; two fit in u64.
    Ok(Some(TokenCounts {
        input: input.unwrap_or_default(),
        output: output.unwrap_or_default() + reasoning,
        reasoning,
        cached_input: cached_input.unwrap_or_default(),
        cache_creation: cache_creation.unwrap_or_default(),
        ..Default::default()
    }))
}

/// Validate the sum before passing SQLite counters to the shared aggregation,
/// which assumes its inputs fit in u64. A corrupt DB must not wrap or panic.
fn check_sum(total: &mut TokenCounts, tokens: &TokenCounts) -> Result<(), ReadError> {
    let sum = u128::from(total.total())
        + u128::from(tokens.input)
        + u128::from(tokens.cached_input)
        + u128::from(tokens.cache_creation)
        + u128::from(tokens.output);
    if sum > u128::from(u64::MAX) || total.reasoning.checked_add(tokens.reasoning).is_none() {
        return Err(ReadError::InvalidUsage);
    }
    total.add(tokens);
    Ok(())
}

#[derive(Default)]
struct Sessions {
    rows: HashMap<String, SessionRow>,
    roots: HashMap<String, String>,
}

impl Sessions {
    fn root(&mut self, db: &Connection, id: &str) -> Result<String, ReadError> {
        let mut id = id.to_owned();
        let mut seen = HashSet::new();
        let root = loop {
            if let Some(root) = self.roots.get(&id) {
                break root.clone();
            }
            if !seen.insert(id.clone()) {
                return Err(ReadError::InvalidUsage);
            }
            let row = db.prepare_cached(SESSION)?.query_row([&id], |row| {
                Ok(SessionRow {
                    parent: row.get(0)?,
                    project: row
                        .get::<_, Option<String>>(1)?
                        .as_deref()
                        .and_then(project_name),
                    created: date(row, 2)?,
                    updated: date(row, 3)?,
                })
            })?;
            let parent = row.parent.clone();
            self.rows.insert(id.clone(), row);
            match parent {
                Some(parent) => id = parent,
                None => break id,
            }
        };
        for child in seen {
            self.roots.insert(child, root.clone());
        }
        Ok(root)
    }
}

#[cfg(test)]
mod tests;

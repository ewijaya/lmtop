//! Claude Code collector. Reads per-project session JSONL logs
//! (`~/.claude/projects/<project-slug>/<session-uuid>.jsonl`), collecting
//! per-request token usage without touching message content.
//!
//! Schema notes (verified against a live installation; see
//! docs/data-sources.md):
//! - Assistant lines carry `message.model` and `message.usage`
//!   (`input_tokens`, `cache_creation_input_tokens`,
//!   `cache_read_input_tokens`, `output_tokens`). Claude's `input_tokens`
//!   EXCLUDES cache tokens, matching the domain mapping directly.
//! - Claude Code writes one line per content block, duplicating the same
//!   `message.id` and usage across consecutive lines — usage must be
//!   counted once per unique message id. The same key also deduplicates
//!   forked/resumed sessions that inherit history lines and archived
//!   copies of a live session.
//! - Sidechain lines (`isSidechain: true`, subagent traffic) carry real
//!   usage and are counted. Subagent transcripts live in nested dirs up to
//!   `<session>/subagents/workflows/wf_*/agent-*.jsonl`, so discovery walks
//!   recursively rather than globbing fixed depths.
//! - `usage.iterations[]`, when non-empty, carries the per-turn truth for
//!   multi-turn (server-side compaction) responses; the top-level counters
//!   silently omit the largest turn, so iterations are summed instead.
//! - Subscription quota lives in `~/.claude.json` under
//!   `cachedUsageUtilization` — a cache written by Claude Code itself with
//!   `five_hour` / `seven_day` percentages, reset times, and model-scoped
//!   limits. Only that subtree is extracted; account identifiers and other
//!   fields in the file are never read into program state.

use super::jsonl::JsonlTail;
use super::store::{UsageStore, project_name};
use super::{Collector, ScanContext};
use crate::aggregation::{self, QuotaSample};
use crate::config::ProviderConfig;
use crate::diagnostics::{DiscoveryInfo, redacted_dirs};
use crate::domain::{
    Capability, CollectorHealth, CollectorStatus, ModelIdentity, Provider, ProviderSnapshot,
    QuotaWindow, QuotaWindowKind, TokenCounts,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct ClaudeCollector {
    claude_home: Option<PathBuf>,
    project_dirs: Vec<PathBuf>,
    tail: JsonlTail,
    store: UsageStore,
    /// Unique API-request keys already counted (message id), shared across
    /// all files so inherited history and archived copies count once.
    seen_messages: HashSet<String>,
    /// Latest assistant timestamp per session, to attribute context size.
    latest_context: HashMap<String, DateTime<Utc>>,
    cli_version: Option<String>,
    parse_errors: u64,
    files_scanned: u64,
    /// Claude Code's own quota cache (`~/.claude.json`), when present.
    quota_file: Option<PathBuf>,
    /// Percentage samples per window key, for burn-velocity estimation.
    quota_samples: BTreeMap<String, Vec<QuotaSample>>,
    /// `fetchedAtMs` of the newest quota cache already sampled.
    last_quota_fetched_ms: Option<i64>,
}

impl ClaudeCollector {
    pub fn from_config(cfg: &ProviderConfig) -> Self {
        let claude_home = directories::UserDirs::new()
            .map(|d| d.home_dir().join(".claude"))
            .filter(|p| p.is_dir());
        let mut defaults = Vec::new();
        if let Some(home) = &claude_home {
            defaults.push(home.join("projects"));
        }
        let project_dirs = super::existing_dirs(&defaults, &cfg.session_paths);
        let quota_file = directories::UserDirs::new()
            .map(|d| d.home_dir().join(".claude.json"))
            .filter(|p| p.is_file());
        ClaudeCollector {
            claude_home,
            project_dirs,
            tail: JsonlTail::new(),
            store: UsageStore::new(Provider::Claude),
            seen_messages: HashSet::new(),
            latest_context: HashMap::new(),
            cli_version: None,
            parse_errors: 0,
            files_scanned: 0,
            quota_file,
            quota_samples: BTreeMap::new(),
            last_quota_fetched_ms: None,
        }
    }

    /// For tests: a collector rooted at explicit project directories.
    pub fn with_dirs(project_dirs: Vec<PathBuf>) -> Self {
        let mut collector = Self::from_config(&ProviderConfig::default());
        collector.claude_home = None;
        collector.project_dirs = project_dirs.into_iter().filter(|p| p.is_dir()).collect();
        collector.quota_file = None;
        collector
    }

    /// For tests: additionally point at an explicit quota cache file.
    pub fn with_dirs_and_quota(project_dirs: Vec<PathBuf>, quota_file: PathBuf) -> Self {
        let mut collector = Self::with_dirs(project_dirs);
        collector.quota_file = quota_file.is_file().then_some(quota_file);
        collector
    }

    pub fn discovery_info(&self) -> DiscoveryInfo {
        // Presence check only; credential files are never opened.
        let auth_present = self
            .claude_home
            .as_ref()
            .map(|h| h.join(".credentials.json").is_file())
            .unwrap_or(false)
            || directories::UserDirs::new()
                .map(|d| d.home_dir().join(".claude.json").is_file())
                .unwrap_or(false);
        DiscoveryInfo {
            provider: Provider::Claude,
            installed: self.claude_home.is_some(),
            session_dirs: redacted_dirs(&self.project_dirs),
            session_files: count_session_files(&self.project_dirs),
            auth_present,
            cli_version: self.cli_version.clone(),
        }
    }

    fn capabilities(&self) -> Vec<Capability> {
        let mut caps = vec![
            Capability::LocalTokenUsage,
            Capability::ActiveSession,
            Capability::CalendarWeekAggregation,
            Capability::ModelBreakdown,
            Capability::History,
        ];
        // Quota comes from Claude Code's own cache file; without it the
        // capability is honestly absent (no credits either way — Claude
        // exposes no local credit balance).
        if self.quota_file.is_some() {
            caps.push(Capability::ProviderQuota);
            caps.push(Capability::ResetTimes);
        }
        caps
    }

    fn ingest_line(&mut self, line: &str) {
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                self.parse_errors += 1;
                return;
            }
        };
        if value.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        if let Some(v) = value.get("version").and_then(Value::as_str) {
            self.cli_version = Some(v.to_string());
        }
        let Some(message) = value.get("message") else {
            return;
        };
        let model_raw = message.get("model").and_then(Value::as_str).unwrap_or("");
        // Synthetic messages are client-generated error placeholders with
        // no real API usage.
        if model_raw == "<synthetic>" {
            return;
        }
        let Some(usage) = message.get("usage").filter(|u| u.is_object()) else {
            return;
        };

        // One count per unique API request: message id first, request id as
        // fallback, line uuid as last resort.
        let dedup_key = message
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| value.get("requestId").and_then(Value::as_str))
            .or_else(|| value.get("uuid").and_then(Value::as_str));
        let Some(dedup_key) = dedup_key else {
            return;
        };
        if !self.seen_messages.insert(dedup_key.to_string()) {
            return;
        }

        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_time);
        let session_id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-session")
            .to_string();
        let tokens = parse_claude_usage(usage);
        let model = (!model_raw.is_empty()).then(|| ModelIdentity::normalize(model_raw));
        let Some(timestamp) = timestamp else {
            self.parse_errors += 1;
            return;
        };

        if !tokens.is_zero() {
            self.store
                .record_event(&session_id, timestamp, model, tokens.clone());
        }

        let record = self.store.session_mut(&session_id);
        if record.project.is_none() {
            record.project = value
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(project_name);
        }
        // Context occupancy after this request ≈ everything the request
        // processed plus what it produced.
        let is_latest = self
            .latest_context
            .get(&session_id)
            .is_none_or(|t| timestamp >= *t);
        if is_latest
            && !value
                .get("isSidechain")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            self.latest_context.insert(session_id.clone(), timestamp);
            record.context_tokens = Some(tokens.total_input() + tokens.output);
        }
    }
}

impl ClaudeCollector {
    /// Read Claude Code's own quota cache (`cachedUsageUtilization` in
    /// `~/.claude.json`). Only that subtree is extracted; nothing else in
    /// the file (account ids, OAuth data, telemetry state) is read into
    /// program state. Absent or unparsable data yields no windows — quota
    /// is never inferred from observed tokens.
    fn read_quota(&mut self, ctx: &ScanContext) -> Vec<QuotaWindow> {
        let Some(path) = &self.quota_file else {
            return Vec::new();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        let Ok(root) = serde_json::from_str::<Value>(&text) else {
            return Vec::new();
        };
        let Some(cache) = root.get("cachedUsageUtilization") else {
            return Vec::new();
        };
        let captured_at = cache
            .get("fetchedAtMs")
            .and_then(Value::as_i64)
            .and_then(chrono::DateTime::from_timestamp_millis)
            .unwrap_or(ctx.now);
        let fetched_ms = cache.get("fetchedAtMs").and_then(Value::as_i64);
        let is_new_sample = fetched_ms != self.last_quota_fetched_ms;
        self.last_quota_fetched_ms = fetched_ms;

        let Some(utilization) = cache.get("utilization") else {
            return Vec::new();
        };

        let mut raw: Vec<RawQuotaWindow> = Vec::new();
        for (name, kind, minutes) in [
            ("five_hour", QuotaWindowKind::FiveHour, Some(300)),
            ("seven_day", QuotaWindowKind::Weekly, Some(10_080)),
        ] {
            if let Some(w) = utilization.get(name).filter(|v| !v.is_null())
                && let Some(pct) = w.get("utilization").and_then(Value::as_f64)
            {
                raw.push(RawQuotaWindow {
                    key: name.to_string(),
                    kind,
                    minutes,
                    percent: pct,
                    resets_at: w
                        .get("resets_at")
                        .and_then(Value::as_str)
                        .and_then(parse_time),
                    scope: None,
                });
            }
        }
        // Model-scoped limits (e.g. a per-model weekly cap). Unscoped
        // entries duplicate the named windows above and are skipped.
        if let Some(limits) = utilization.get("limits").and_then(Value::as_array) {
            for limit in limits {
                let Some(scope_model) = limit
                    .get("scope")
                    .and_then(|s| s.get("model"))
                    .and_then(|m| m.get("display_name"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                let Some(pct) = limit.get("percent").and_then(Value::as_f64) else {
                    continue;
                };
                let group = limit.get("group").and_then(Value::as_str).unwrap_or("?");
                let (kind, minutes) = match group {
                    "weekly" => (QuotaWindowKind::Weekly, Some(10_080)),
                    "session" => (QuotaWindowKind::FiveHour, Some(300)),
                    _ => (QuotaWindowKind::Unknown, None),
                };
                raw.push(RawQuotaWindow {
                    key: format!("scoped:{group}:{scope_model}"),
                    kind,
                    minutes,
                    percent: pct,
                    resets_at: limit
                        .get("resets_at")
                        .and_then(Value::as_str)
                        .and_then(parse_time),
                    scope: Some(scope_model.to_string()),
                });
            }
        }

        let mut windows = Vec::new();
        for rw in raw {
            let samples = self.quota_samples.entry(rw.key).or_default();
            if is_new_sample {
                samples.push(QuotaSample {
                    captured_at,
                    used_percent: rw.percent,
                });
                if samples.len() > 2000 {
                    let excess = samples.len() - 2000;
                    samples.drain(..excess);
                }
            }
            let (burn_per_hour, projected_exhaustion) =
                aggregation::project_quota(samples, ctx.now);
            windows.push(QuotaWindow {
                kind: rw.kind,
                used_percent: rw.percent,
                window_minutes: rw.minutes,
                resets_at: rw.resets_at,
                captured_at,
                scope: rw.scope,
                burn_per_hour,
                projected_exhaustion,
            });
        }
        windows
    }
}

/// One quota window as parsed from the cache, before burn estimation.
struct RawQuotaWindow {
    /// Stable sample-series key (window name or scoped identity).
    key: String,
    kind: QuotaWindowKind,
    minutes: Option<u64>,
    percent: f64,
    resets_at: Option<DateTime<Utc>>,
    scope: Option<String>,
}

impl Collector for ClaudeCollector {
    fn provider(&self) -> Provider {
        Provider::Claude
    }

    fn scan(&mut self, ctx: &ScanContext) -> ProviderSnapshot {
        if self.project_dirs.is_empty() {
            return ProviderSnapshot::empty(
                Provider::Claude,
                CollectorHealth::unavailable("no Claude Code projects directory found"),
            );
        }

        let cutoff = ctx.retention_cutoff();
        let files = discover_session_files(&self.project_dirs, cutoff);
        self.files_scanned = files.len() as u64;
        for path in files {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if !self.tail.needs_read(&path, meta.len()) {
                continue;
            }
            match self.tail.read_new_lines(&path) {
                Ok(outcome) => {
                    for line in outcome.lines {
                        self.ingest_line(&line);
                    }
                }
                Err(_) => {
                    self.parse_errors += 1;
                }
            }
        }
        self.store.trim(ctx);
        let quota_windows = self.read_quota(ctx);

        let status = if self.parse_errors > 0 {
            CollectorStatus::Degraded
        } else {
            CollectorStatus::Ok
        };
        let health = CollectorHealth {
            status,
            message: (self.parse_errors > 0)
                .then(|| format!("{} unparsable lines skipped", self.parse_errors)),
            last_scan: Some(ctx.now),
            files_scanned: self.files_scanned,
            parse_errors: self.parse_errors,
        };
        self.store
            .build_snapshot(ctx, self.capabilities(), health, quota_windows, None)
    }
}

/// Map Claude usage to domain semantics. Claude's `input_tokens` already
/// excludes cache tokens, so fields map 1:1; unknown `*_tokens` categories
/// are preserved.
///
/// When `usage.iterations[]` is non-empty it is summed instead of using the
/// top-level counters: for multi-turn responses (server-side compaction)
/// the top level silently omits the largest turn (verified: top-level
/// output equals `sum(iterations) - max(iterations)`), while for the
/// single-turn case `iterations[0]` equals the top level, so summing is
/// always safe.
fn parse_claude_usage(usage: &Value) -> TokenCounts {
    if let Some(iterations) = usage
        .get("iterations")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
    {
        let mut total = TokenCounts::default();
        for iteration in iterations {
            total.add(&parse_claude_usage_flat(iteration));
        }
        // Unknown categories only exist at the top level; carry them over.
        for (key, n) in parse_claude_usage_flat(usage).other {
            total.other.entry(key).or_insert(n);
        }
        return total;
    }
    parse_claude_usage_flat(usage)
}

fn parse_claude_usage_flat(usage: &Value) -> TokenCounts {
    let mut other = BTreeMap::new();
    if let Some(map) = usage.as_object() {
        for (key, value) in map {
            if key.ends_with("_tokens")
                && !matches!(
                    key.as_str(),
                    "input_tokens"
                        | "cache_creation_input_tokens"
                        | "cache_read_input_tokens"
                        | "output_tokens"
                )
                && let Some(n) = value.as_u64()
            {
                other.insert(key.clone(), n);
            }
        }
    }
    TokenCounts {
        input: read_u64(usage, "input_tokens"),
        cached_input: read_u64(usage, "cache_read_input_tokens"),
        cache_creation: read_u64(usage, "cache_creation_input_tokens"),
        output: read_u64(usage, "output_tokens"),
        reasoning: 0,
        other,
    }
}

fn read_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Transcripts live at several depths: main sessions at
/// `<projects>/<slug>/<session>.jsonl`, subagent transcripts under
/// `<slug>/<session>/subagents/`, and workflow agents another two levels
/// down (`subagents/workflows/wf_*/agent-*.jsonl`) — hence a recursive
/// walk with a generous depth bound instead of fixed-depth globs.
fn discover_session_files(dirs: &[PathBuf], modified_since: DateTime<Utc>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in dirs {
        walk(dir, &mut files, modified_since, 0);
    }
    files.sort();
    files
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, modified_since: DateTime<Utc>, depth: u8) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out, modified_since, depth + 1);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            let recent = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| DateTime::<Utc>::from(t) >= modified_since)
                .unwrap_or(true);
            if recent {
                out.push(path);
            }
        }
    }
}

fn count_session_files(dirs: &[PathBuf]) -> u64 {
    let mut files = Vec::new();
    for dir in dirs {
        walk(dir, &mut files, DateTime::<Utc>::MIN_UTC, 0);
    }
    files.len() as u64
}

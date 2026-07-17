//! Codex CLI collector. Reads rollout JSONL session logs
//! (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`), converting Codex's
//! cumulative token counters to deltas and extracting the provider-reported
//! rate-limit snapshots embedded in `token_count` events.
//!
//! Schema notes (verified against a live installation; see
//! docs/data-sources.md):
//! - `session_meta` line: `payload.id`, `payload.timestamp`, `payload.cwd`,
//!   `payload.cli_version`.
//! - `turn_context` line: `payload.model`, changing mid-session on model
//!   switches.
//! - `event_msg` / `payload.type == "token_count"`:
//!   `payload.info.total_token_usage` (cumulative), `.last_token_usage`
//!   (per turn), `.model_context_window`, and `payload.rate_limits` with
//!   `primary` / `secondary` windows (`used_percent`, `window_minutes`,
//!   `resets_at` or `resets_in_seconds`). Codex `input_tokens` INCLUDES
//!   cached tokens; the domain's `input` excludes them, so the mapping
//!   subtracts.
//! - Windows are classified by duration, never by primary/secondary
//!   position: live data shows `primary` can be the weekly window with
//!   `secondary` absent.

use super::jsonl::JsonlTail;
use super::live_quota::LiveQuota;
use super::store::{UsageStore, project_name};
use super::{Collector, ScanContext};
use crate::aggregation::{self, QuotaSample};
use crate::config::ProviderConfig;
use crate::diagnostics::{DiscoveryInfo, redacted_dirs};
use crate::domain::{
    Capability, CollectorHealth, CollectorStatus, Credits, ModelIdentity, Provider,
    ProviderSnapshot, QuotaWindow, QuotaWindowKind, TokenCounts,
};
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Rate-limit samples older than this no longer inform the burn trend.
const QUOTA_SAMPLE_RETENTION_HOURS: i64 = 7 * 24;

pub struct CodexCollector {
    codex_home: Option<PathBuf>,
    session_dirs: Vec<PathBuf>,
    tail: JsonlTail,
    store: UsageStore,
    files: HashMap<PathBuf, FileState>,
    /// session id -> file that owns it (first claim wins), so an archived
    /// copy of an active session is never double counted.
    session_owner: HashMap<String, PathBuf>,
    /// Quota samples per window identity (window_minutes bucket).
    quota_samples: BTreeMap<Option<u64>, Vec<QuotaSample>>,
    /// Latest observed state per window identity.
    latest_window: BTreeMap<Option<u64>, LatestWindow>,
    /// Opt-in live quota fetcher (`network_quota = true`); its samples run
    /// through the same ingestion path as rollout-file snapshots.
    live: Option<LiveQuota>,
    credits: Option<Credits>,
    cli_version: Option<String>,
    parse_errors: u64,
    files_scanned: u64,
}

#[derive(Debug, Clone)]
struct LatestWindow {
    used_percent: f64,
    resets_at: Option<DateTime<Utc>>,
    captured_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct FileState {
    session_id: Option<String>,
    /// True when another file already owns this session id.
    duplicate: bool,
    model: Option<ModelIdentity>,
    last_cumulative: Option<TokenCounts>,
    saw_token_count: bool,
}

impl CodexCollector {
    pub fn from_config(cfg: &ProviderConfig) -> Self {
        let codex_home = directories::UserDirs::new()
            .map(|d| d.home_dir().join(".codex"))
            .filter(|p| p.is_dir());
        let mut defaults = Vec::new();
        if let Some(home) = &codex_home {
            defaults.push(home.join("sessions"));
            // Archived sessions, when present, may hold copies of rollout
            // files; the session-owner map deduplicates them.
            defaults.push(home.join("archived_sessions"));
        }
        let session_dirs = super::existing_dirs(&defaults, &cfg.session_paths);
        CodexCollector {
            codex_home,
            session_dirs,
            tail: JsonlTail::new(),
            store: UsageStore::new(Provider::Codex),
            files: HashMap::new(),
            session_owner: HashMap::new(),
            quota_samples: BTreeMap::new(),
            latest_window: BTreeMap::new(),
            live: cfg.network_quota.then(LiveQuota::for_codex),
            credits: None,
            cli_version: None,
            parse_errors: 0,
            files_scanned: 0,
        }
    }

    /// For tests: a collector rooted at explicit session directories.
    pub fn with_dirs(session_dirs: Vec<PathBuf>) -> Self {
        let mut collector = Self::from_config(&ProviderConfig::default());
        collector.codex_home = None;
        collector.session_dirs = session_dirs.into_iter().filter(|p| p.is_dir()).collect();
        collector.live = None;
        collector
    }

    pub fn discovery_info(&self) -> DiscoveryInfo {
        let auth_present = self
            .codex_home
            .as_ref()
            .map(|h| h.join("auth.json").is_file())
            .unwrap_or(false);
        DiscoveryInfo {
            provider: Provider::Codex,
            installed: self.codex_home.is_some(),
            session_dirs: redacted_dirs(&self.session_dirs),
            session_files: count_jsonl_files(&self.session_dirs),
            auth_present,
            cli_version: self.cli_version.clone(),
        }
    }

    fn capabilities() -> Vec<Capability> {
        vec![
            Capability::LocalTokenUsage,
            Capability::ActiveSession,
            Capability::CalendarWeekAggregation,
            Capability::ModelBreakdown,
            Capability::ProviderQuota,
            Capability::Credits,
            Capability::ResetTimes,
            Capability::History,
        ]
    }

    fn ingest_line(&mut self, path: &Path, line: &str, ctx: &ScanContext) {
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                self.parse_errors += 1;
                return;
            }
        };
        let line_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_time);
        let payload = value.get("payload").unwrap_or(&Value::Null);

        match line_type {
            "session_meta" => self.ingest_session_meta(path, payload),
            "turn_context" => {
                if let Some(model) = payload.get("model").and_then(Value::as_str) {
                    self.files.entry(path.to_path_buf()).or_default().model =
                        Some(ModelIdentity::normalize(model));
                }
            }
            "event_msg" if payload.get("type").and_then(Value::as_str) == Some("token_count") => {
                self.ingest_token_count(path, payload, timestamp, ctx);
            }
            _ => {}
        }
    }

    fn ingest_session_meta(&mut self, path: &Path, payload: &Value) {
        if let Some(v) = payload.get("cli_version").and_then(Value::as_str) {
            self.cli_version = Some(v.to_string());
        }
        let state = self.files.entry(path.to_path_buf()).or_default();
        if let Some(id) = payload.get("id").and_then(Value::as_str) {
            match self.session_owner.get(id) {
                Some(owner) if owner != path => {
                    // Another file (e.g. the live copy of an archived
                    // session) already owns this session's usage.
                    state.duplicate = true;
                    return;
                }
                _ => {
                    self.session_owner
                        .insert(id.to_string(), path.to_path_buf());
                    state.session_id = Some(id.to_string());
                }
            }
        }
        let session_id = state.session_id.clone();
        let project = payload
            .get("cwd")
            .and_then(Value::as_str)
            .and_then(project_name);
        let started = payload
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_time);
        if let Some(id) = session_id {
            let record = self.store.session_mut(&id);
            if project.is_some() {
                record.project = project;
            }
            if record.started_at.is_none() {
                record.started_at = started;
            }
        }
    }

    fn ingest_token_count(
        &mut self,
        path: &Path,
        payload: &Value,
        timestamp: Option<DateTime<Utc>>,
        ctx: &ScanContext,
    ) {
        let timestamp = timestamp.unwrap_or(ctx.now);

        // Rate limits are provider-authoritative account state; capture
        // them even from duplicate session files.
        if let Some(rate_limits) = payload.get("rate_limits").filter(|v| !v.is_null()) {
            self.ingest_rate_limits(rate_limits, timestamp);
        }

        let state = self.files.entry(path.to_path_buf()).or_default();
        if state.duplicate {
            return;
        }
        let Some(info) = payload.get("info").filter(|v| !v.is_null()) else {
            return;
        };
        let Some(total) = info.get("total_token_usage").map(parse_codex_usage) else {
            return;
        };
        let last = info.get("last_token_usage").map(parse_codex_usage);

        // Cumulative -> delta. For the first counter in a file, decide
        // whether the cumulative history was inherited (a forked/resumed
        // session whose log starts from an old counter): a brand-new
        // session's first counter equals its per-turn counter. If they
        // differ, the total includes inherited history that another file
        // already accounted for — count only the per-turn part.
        let delta = match (&state.last_cumulative, &last) {
            (Some(prev), _) => TokenCounts::saturating_delta(&total, prev),
            (None, Some(last_usage)) if !state.saw_token_count && last_usage != &total => {
                last_usage.clone()
            }
            (None, _) => total.clone(),
        };
        state.last_cumulative = Some(total);
        state.saw_token_count = true;
        let model = state.model.clone();
        let session_id = state
            .session_id
            .clone()
            .unwrap_or_else(|| fallback_session_id(path));

        if !delta.is_zero() {
            self.store
                .record_event(&session_id, timestamp, model, delta);
        }

        // Context occupancy: the last request's input+output is the best
        // local estimate of what currently sits in the context window.
        let record = self.store.session_mut(&session_id);
        if let Some(last) = info.get("last_token_usage") {
            let input = read_u64(last, "input_tokens");
            let output = read_u64(last, "output_tokens");
            if input + output > 0 {
                record.context_tokens = Some(input + output);
            }
        }
        if let Some(window) = info.get("model_context_window").and_then(Value::as_u64) {
            record.context_window = Some(window);
        }
        if record.last_activity.is_none_or(|t| timestamp > t) {
            record.last_activity = Some(timestamp);
        }
    }

    fn ingest_rate_limits(&mut self, rate_limits: &Value, captured_at: DateTime<Utc>) {
        for key in ["primary", "secondary"] {
            let Some(window) = rate_limits.get(key).filter(|v| !v.is_null()) else {
                continue;
            };
            let Some(used_percent) = window.get("used_percent").and_then(Value::as_f64) else {
                continue;
            };
            let window_minutes = window.get("window_minutes").and_then(Value::as_u64);
            let resets_at = window
                .get("resets_at")
                .and_then(Value::as_i64)
                .and_then(|s| Utc.timestamp_opt(s, 0).single())
                .or_else(|| {
                    window
                        .get("resets_in_seconds")
                        .and_then(Value::as_i64)
                        .map(|s| captured_at + chrono::Duration::seconds(s))
                });
            let entry = self
                .latest_window
                .entry(window_minutes)
                .or_insert_with(|| LatestWindow {
                    used_percent,
                    resets_at,
                    captured_at,
                });
            if captured_at >= entry.captured_at {
                *entry = LatestWindow {
                    used_percent,
                    resets_at,
                    captured_at,
                };
            }
            self.quota_samples
                .entry(window_minutes)
                .or_default()
                .push(QuotaSample {
                    captured_at,
                    used_percent,
                });
        }
        // Credits balance, when the plan reports one (number or object).
        if let Some(v) = rate_limits.get("credits").filter(|v| !v.is_null()) {
            let balance = v
                .as_f64()
                .or_else(|| v.get("balance").and_then(Value::as_f64))
                .or_else(|| {
                    v.get("balance")
                        .and_then(Value::as_str)
                        .and_then(|s| s.parse().ok())
                });
            if let Some(balance) = balance
                && self
                    .credits
                    .as_ref()
                    .is_none_or(|c| captured_at >= c.captured_at)
            {
                self.credits = Some(Credits {
                    balance,
                    captured_at,
                });
            }
        }
    }

    fn build_quota_windows(&mut self, ctx: &ScanContext) -> Vec<QuotaWindow> {
        let cutoff = ctx.now - chrono::Duration::hours(QUOTA_SAMPLE_RETENTION_HOURS);
        for samples in self.quota_samples.values_mut() {
            samples.retain(|s| s.captured_at >= cutoff);
            // Keep memory bounded even under constant activity.
            if samples.len() > 2000 {
                let excess = samples.len() - 2000;
                samples.drain(..excess);
            }
        }
        let mut windows = Vec::new();
        for (window_minutes, latest) in &self.latest_window {
            let samples = self
                .quota_samples
                .get(window_minutes)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let projection = aggregation::project_quota(samples, ctx.now);
            windows.push(QuotaWindow {
                kind: QuotaWindowKind::from_window_minutes(*window_minutes),
                used_percent: latest.used_percent,
                window_minutes: *window_minutes,
                resets_at: latest.resets_at,
                captured_at: latest.captured_at,
                scope: None,
                burn_per_hour: projection.burn_per_hour,
                projected_exhaustion: projection.projected_exhaustion,
                trend_confidence: projection.confidence,
            });
        }
        windows
    }
}

impl Collector for CodexCollector {
    fn provider(&self) -> Provider {
        Provider::Codex
    }

    fn scan(&mut self, ctx: &ScanContext) -> ProviderSnapshot {
        // Without session logs there is nothing to observe locally — but the
        // opt-in live quota fetch can still report account state.
        if self.session_dirs.is_empty() && self.live.is_none() {
            return ProviderSnapshot::empty(
                Provider::Codex,
                CollectorHealth::unavailable("no Codex session directory found"),
            );
        }

        let cutoff = ctx.retention_cutoff();
        let files = discover_jsonl_files(&self.session_dirs, cutoff);
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
                    if outcome.truncated {
                        // Rotated/replaced file: forget its parse state so
                        // counters restart from a clean baseline.
                        self.files.remove(&path);
                    }
                    for line in outcome.lines {
                        self.ingest_line(&path, &line, ctx);
                    }
                }
                Err(_) => {
                    self.parse_errors += 1;
                }
            }
        }
        self.store.trim(ctx);

        // Live quota, when enabled: each fresh response is normalized into
        // the rollout `rate_limits` shape and ingested exactly once.
        if let Some(live) = self.live.as_mut()
            && let Some(rate_limits) = live.codex_rate_limits(ctx.now)
        {
            self.ingest_rate_limits(&rate_limits, ctx.now);
        }

        let quota_windows = self.build_quota_windows(ctx);
        let mut problems = Vec::new();
        if self.parse_errors > 0 {
            problems.push(format!("{} unparsable lines skipped", self.parse_errors));
        }
        if let Some(err) = self.live.as_ref().and_then(|l| l.last_error.as_ref()) {
            problems.push(format!("live quota: {err} — using local data"));
        }
        let status = if problems.is_empty() {
            CollectorStatus::Ok
        } else {
            CollectorStatus::Degraded
        };
        let health = CollectorHealth {
            status,
            message: (!problems.is_empty()).then(|| problems.join(" · ")),
            last_scan: Some(ctx.now),
            files_scanned: self.files_scanned,
            parse_errors: self.parse_errors,
        };
        self.store.build_snapshot(
            ctx,
            Self::capabilities(),
            health,
            quota_windows,
            self.credits.clone(),
        )
    }
}

/// Map Codex usage counters to domain semantics. Codex `input_tokens`
/// includes cached tokens; the domain's `input` excludes them.
fn parse_codex_usage(usage: &Value) -> TokenCounts {
    let input_incl_cached = read_u64(usage, "input_tokens");
    let cached = read_u64(usage, "cached_input_tokens");
    let mut other = BTreeMap::new();
    if let Some(map) = usage.as_object() {
        for (key, value) in map {
            if key.ends_with("_tokens")
                && !matches!(
                    key.as_str(),
                    "input_tokens"
                        | "cached_input_tokens"
                        | "output_tokens"
                        | "reasoning_output_tokens"
                        | "total_tokens"
                )
                && let Some(n) = value.as_u64()
            {
                other.insert(key.clone(), n);
            }
        }
    }
    TokenCounts {
        input: input_incl_cached.saturating_sub(cached),
        cached_input: cached,
        cache_creation: 0,
        output: read_u64(usage, "output_tokens"),
        reasoning: read_u64(usage, "reasoning_output_tokens"),
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

/// Sessions whose meta line is missing still get a stable identity from
/// the file name.
fn fallback_session_id(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn discover_jsonl_files(dirs: &[PathBuf], modified_since: DateTime<Utc>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in dirs {
        walk(dir, &mut files, modified_since, 0);
    }
    files.sort();
    files
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, modified_since: DateTime<Utc>, depth: u8) {
    if depth > 6 {
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

fn count_jsonl_files(dirs: &[PathBuf]) -> u64 {
    let mut files = Vec::new();
    for dir in dirs {
        walk(dir, &mut files, DateTime::<Utc>::MIN_UTC, 0);
    }
    files.len() as u64
}

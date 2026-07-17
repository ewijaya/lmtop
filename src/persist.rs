//! Durable usage history: per-minute token-rate buckets and quota samples,
//! appended as JSONL in the platform data directory and reloaded at
//! startup. This is what lets the chart and quota timeline reach back
//! before the current process started.
//!
//! Privacy: entries hold provider names, timestamps, token counts, and
//! quota percentages — never session ids, project names, or content.

use crate::domain::{HistorySample, Provider, ProviderSnapshot, QuotaWindowKind};
use chrono::{DateTime, Duration, DurationRound, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

/// One persisted line. Short field names keep the file compact; it grows by
/// roughly a few hundred bytes per active minute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum HistoryEntry {
    Rate {
        at: DateTime<Utc>,
        p: Provider,
        #[serde(rename = "in")]
        input: u64,
        out: u64,
    },
    Quota {
        at: DateTime<Utc>,
        p: Provider,
        kind: QuotaWindowKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        pct: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resets: Option<DateTime<Utc>>,
    },
}

/// A quota percentage observed at a point in time, for the timeline chart.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaPoint {
    pub at: DateTime<Utc>,
    pub kind: QuotaWindowKind,
    pub scope: Option<String>,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct ProviderHistory {
    /// Completed per-minute buckets keyed by bucket start.
    rate: BTreeMap<DateTime<Utc>, (u64, u64)>,
    quota: Vec<QuotaPoint>,
}

/// Append-only history store with the full retained window in memory.
#[derive(Debug)]
pub struct HistoryStore {
    path: PathBuf,
    providers: BTreeMap<Provider, ProviderHistory>,
    retention: Duration,
}

impl HistoryStore {
    /// Default location: `<data dir>/history.jsonl`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", crate::branding::APP_DIR)
            .map(|d| d.data_local_dir().join("history.jsonl"))
    }

    /// Open (creating the directory if needed), load the retained window,
    /// and compact the file if pruning dropped anything. Errors are treated
    /// as "no history" — the dashboard must never fail to start over a
    /// corrupt or unwritable history file.
    pub fn open(path: PathBuf, retention_days: u64, now: DateTime<Utc>) -> Option<HistoryStore> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        let retention = Duration::days(retention_days.max(1) as i64);
        let cutoff = now - retention;
        let mut store = HistoryStore {
            path,
            providers: BTreeMap::new(),
            retention,
        };
        let mut kept: Vec<HistoryEntry> = Vec::new();
        let mut dropped = 0usize;
        if let Ok(text) = std::fs::read_to_string(&store.path) {
            for line in text.lines() {
                let Ok(entry) = serde_json::from_str::<HistoryEntry>(line) else {
                    dropped += 1; // unreadable line: drop on compaction
                    continue;
                };
                let at = match &entry {
                    HistoryEntry::Rate { at, .. } | HistoryEntry::Quota { at, .. } => *at,
                };
                if at < cutoff {
                    dropped += 1;
                    continue;
                }
                store.absorb(entry.clone());
                kept.push(entry);
            }
        }
        if dropped > 0 {
            store.rewrite(&kept);
        }
        Some(store)
    }

    fn absorb(&mut self, entry: HistoryEntry) {
        match entry {
            HistoryEntry::Rate { at, p, input, out } => {
                self.providers.entry(p).or_default().rate.insert(at, (input, out));
            }
            HistoryEntry::Quota {
                at,
                p,
                kind,
                scope,
                pct,
                resets,
            } => {
                self.providers.entry(p).or_default().quota.push(QuotaPoint {
                    at,
                    kind,
                    scope,
                    used_percent: pct,
                    resets_at: resets,
                });
            }
        }
    }

    fn rewrite(&self, entries: &[HistoryEntry]) {
        let tmp = self.path.with_extension("jsonl.tmp");
        let Ok(mut f) = std::fs::File::create(&tmp) else {
            return;
        };
        for e in entries {
            if let Ok(line) = serde_json::to_string(e) {
                let _ = writeln!(f, "{line}");
            }
        }
        let _ = f.flush();
        let _ = std::fs::rename(&tmp, &self.path);
    }

    fn append(&mut self, entries: &[HistoryEntry]) {
        if entries.is_empty() {
            return;
        }
        let Ok(mut f) = std::fs::File::options()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            return;
        };
        for e in entries {
            if let Ok(line) = serde_json::to_string(e) {
                let _ = writeln!(f, "{line}");
            }
            self.absorb(e.clone());
        }
        let _ = f.flush();
    }

    /// Record everything new in a provider snapshot: completed rate-minute
    /// buckets not yet persisted, and quota readings whose capture time or
    /// value changed since the last persisted point for that window.
    pub fn record(&mut self, snap: &ProviderSnapshot, now: DateTime<Utc>) {
        let mut new_entries: Vec<HistoryEntry> = Vec::new();
        let current_minute = now
            .duration_trunc(Duration::minutes(1))
            .unwrap_or(now);
        {
            let ph = self.providers.entry(snap.provider).or_default();
            let last_rate = ph.rate.keys().next_back().copied();
            for sample in &snap.history {
                // Skip the in-progress minute (it may still grow) and
                // anything already persisted.
                if sample.at >= current_minute {
                    continue;
                }
                if last_rate.is_some_and(|t| sample.at <= t) {
                    continue;
                }
                if sample.input_tokens == 0 && sample.output_tokens == 0 {
                    continue; // idle minutes are implicit
                }
                new_entries.push(HistoryEntry::Rate {
                    at: sample.at,
                    p: snap.provider,
                    input: sample.input_tokens,
                    out: sample.output_tokens,
                });
            }
            for w in &snap.quota_windows {
                let is_new = ph
                    .quota
                    .iter()
                    .rev()
                    .find(|q| q.kind == w.kind && q.scope == w.scope)
                    .is_none_or(|last| {
                        last.at != w.captured_at || last.used_percent != w.used_percent
                    });
                if is_new {
                    new_entries.push(HistoryEntry::Quota {
                        at: w.captured_at,
                        p: snap.provider,
                        kind: w.kind.clone(),
                        scope: w.scope.clone(),
                        pct: w.used_percent,
                        resets: w.resets_at,
                    });
                }
            }
        }
        self.append(&new_entries);
    }

    /// Per-minute rate samples for a provider in `[from, to)`, merged from
    /// the persisted record. Idle minutes are not filled in.
    pub fn rate_series(
        &self,
        provider: Provider,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Vec<HistorySample> {
        let Some(ph) = self.providers.get(&provider) else {
            return Vec::new();
        };
        ph.rate
            .range(from..to)
            .map(|(at, (input, output))| HistorySample {
                at: *at,
                input_tokens: *input,
                output_tokens: *output,
            })
            .collect()
    }

    /// Quota points for a provider in `[from, to)`, oldest first.
    pub fn quota_series(
        &self,
        provider: Provider,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Vec<&QuotaPoint> {
        let Some(ph) = self.providers.get(&provider) else {
            return Vec::new();
        };
        ph.quota
            .iter()
            .filter(|q| q.at >= from && q.at < to)
            .collect()
    }

    /// Oldest persisted timestamp across providers, bounding how far the
    /// history view can pan.
    pub fn oldest(&self) -> Option<DateTime<Utc>> {
        self.providers
            .values()
            .flat_map(|ph| {
                ph.rate
                    .keys()
                    .next()
                    .copied()
                    .into_iter()
                    .chain(ph.quota.first().map(|q| q.at))
            })
            .min()
    }

    pub fn retention(&self) -> Duration {
        self.retention
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CollectorHealth, CollectorStatus, QuotaWindow};

    fn snapshot_with(
        history: Vec<HistorySample>,
        quota_windows: Vec<QuotaWindow>,
    ) -> ProviderSnapshot {
        let mut s = ProviderSnapshot::empty(
            Provider::Codex,
            CollectorHealth {
                status: CollectorStatus::Ok,
                message: None,
                last_scan: None,
                files_scanned: 0,
                parse_errors: 0,
            },
        );
        s.history = history;
        s.quota_windows = quota_windows;
        s
    }

    fn window(pct: f64, captured: &str) -> QuotaWindow {
        QuotaWindow {
            kind: QuotaWindowKind::FiveHour,
            used_percent: pct,
            window_minutes: Some(300),
            resets_at: None,
            captured_at: captured.parse().unwrap(),
            scope: None,
            burn_per_hour: None,
            projected_exhaustion: None,
            trend_confidence: None,
        }
    }

    fn sample(at: &str, input: u64, output: u64) -> HistorySample {
        HistorySample {
            at: at.parse().unwrap(),
            input_tokens: input,
            output_tokens: output,
        }
    }

    #[test]
    fn record_and_reload_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lmtop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.jsonl");
        let _ = std::fs::remove_file(&path);
        let now: DateTime<Utc> = "2026-07-17T10:05:30Z".parse().unwrap();

        let mut store = HistoryStore::open(path.clone(), 30, now).unwrap();
        let snap = snapshot_with(
            vec![
                sample("2026-07-17T10:03:00Z", 100, 20),
                sample("2026-07-17T10:04:00Z", 200, 40),
                // In-progress minute: must not persist.
                sample("2026-07-17T10:05:00Z", 999, 999),
            ],
            vec![window(42.0, "2026-07-17T10:04:00Z")],
        );
        store.record(&snap, now);
        // Re-recording the same snapshot adds nothing new.
        store.record(&snap, now);

        let reloaded = HistoryStore::open(path.clone(), 30, now).unwrap();
        let rates = reloaded.rate_series(
            Provider::Codex,
            "2026-07-17T00:00:00Z".parse().unwrap(),
            now,
        );
        assert_eq!(rates.len(), 2);
        assert_eq!(rates[1].input_tokens, 200);
        let quota = reloaded.quota_series(
            Provider::Codex,
            "2026-07-17T00:00:00Z".parse().unwrap(),
            now,
        );
        assert_eq!(quota.len(), 1);
        assert_eq!(quota[0].used_percent, 42.0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn quota_changes_are_appended_and_duplicates_skipped() {
        let dir = std::env::temp_dir().join(format!("lmtop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("quota.jsonl");
        let _ = std::fs::remove_file(&path);
        let now: DateTime<Utc> = "2026-07-17T10:05:30Z".parse().unwrap();

        let mut store = HistoryStore::open(path.clone(), 30, now).unwrap();
        store.record(
            &snapshot_with(vec![], vec![window(42.0, "2026-07-17T10:00:00Z")]),
            now,
        );
        store.record(
            &snapshot_with(vec![], vec![window(42.0, "2026-07-17T10:00:00Z")]),
            now,
        );
        store.record(
            &snapshot_with(vec![], vec![window(43.5, "2026-07-17T10:05:00Z")]),
            now,
        );
        let quota = store.quota_series(
            Provider::Codex,
            "2026-07-17T00:00:00Z".parse().unwrap(),
            now,
        );
        assert_eq!(quota.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn old_entries_are_pruned_on_open() {
        let dir = std::env::temp_dir().join(format!("lmtop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prune.jsonl");
        let _ = std::fs::remove_file(&path);
        let now: DateTime<Utc> = "2026-07-17T10:05:30Z".parse().unwrap();

        let mut store = HistoryStore::open(path.clone(), 7, now).unwrap();
        store.append(&[
            HistoryEntry::Rate {
                at: "2026-07-01T10:00:00Z".parse().unwrap(), // 16 days old
                p: Provider::Codex,
                input: 1,
                out: 1,
            },
            HistoryEntry::Rate {
                at: "2026-07-16T10:00:00Z".parse().unwrap(),
                p: Provider::Codex,
                input: 2,
                out: 2,
            },
        ]);
        let reloaded = HistoryStore::open(path.clone(), 7, now).unwrap();
        let rates = reloaded.rate_series(
            Provider::Codex,
            "2026-01-01T00:00:00Z".parse().unwrap(),
            now,
        );
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].input_tokens, 2);
        // File itself was compacted.
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_lines_are_tolerated() {
        let dir = std::env::temp_dir().join(format!("lmtop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.jsonl");
        std::fs::write(&path, "not json\n{\"t\":\"rate\",\"at\":\"2026-07-17T10:00:00Z\",\"p\":\"codex\",\"in\":5,\"out\":6}\n").unwrap();
        let now: DateTime<Utc> = "2026-07-17T10:05:30Z".parse().unwrap();
        let store = HistoryStore::open(path.clone(), 30, now).unwrap();
        let rates = store.rate_series(
            Provider::Codex,
            "2026-01-01T00:00:00Z".parse().unwrap(),
            now,
        );
        assert_eq!(rates.len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}

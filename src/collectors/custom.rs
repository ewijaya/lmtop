//! User-defined provider collector: ingests a documented JSON schema from a
//! file or command output, so providers lmtop has no built-in collector for
//! (Gemini, Ollama, OpenRouter, …) can be wired in with a small script.
//!
//! Schema (all fields optional except session ids):
//!
//! ```json
//! {
//!   "captured_at": "2026-07-17T10:00:00Z",
//!   "quota_windows": [
//!     {"used_percent": 41.5, "window_minutes": 300,
//!      "resets_at": "2026-07-17T14:00:00Z", "scope": null}
//!   ],
//!   "credits": 12.5,
//!   "sessions": [
//!     {"id": "abc", "model": "gemini-2.5-pro", "project": "myapp",
//!      "last_activity": "2026-07-17T09:59:00Z",
//!      "tokens": {"input": 100, "cached_input": 0, "output": 50},
//!      "context_tokens": 40000, "context_window": 1000000}
//!   ]
//! }
//! ```
//!
//! Session token counts are cumulative; lmtop computes deltas between scans
//! (feeding the week aggregate and rate chart) and never trusts a shrinking
//! counter. Quota percentages get the same burn-trend treatment as the
//! built-in providers.

use super::store::UsageStore;
use super::{Collector, ScanContext};
use crate::aggregation::{QuotaSample, project_quota};
use crate::config::CustomProviderConfig;
use crate::domain::{
    Capability, CollectorHealth, CollectorStatus, Credits, ModelIdentity, Provider,
    ProviderSnapshot, QuotaWindow, QuotaWindowKind, TokenCounts,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalReport {
    #[serde(default)]
    captured_at: Option<DateTime<Utc>>,
    #[serde(default)]
    quota_windows: Vec<ExternalWindow>,
    #[serde(default)]
    credits: Option<f64>,
    #[serde(default)]
    sessions: Vec<ExternalSession>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalWindow {
    used_percent: f64,
    #[serde(default)]
    window_minutes: Option<u64>,
    #[serde(default)]
    resets_at: Option<DateTime<Utc>>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalSession {
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_activity: Option<DateTime<Utc>>,
    #[serde(default)]
    tokens: ExternalTokens,
    #[serde(default)]
    context_tokens: Option<u64>,
    #[serde(default)]
    context_window: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalTokens {
    #[serde(default)]
    input: u64,
    #[serde(default)]
    cached_input: u64,
    #[serde(default)]
    cache_creation: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    reasoning: u64,
}

impl From<&ExternalTokens> for TokenCounts {
    fn from(t: &ExternalTokens) -> TokenCounts {
        TokenCounts {
            input: t.input,
            cached_input: t.cached_input,
            cache_creation: t.cache_creation,
            output: t.output,
            reasoning: t.reasoning,
            unattributed: 0,
            other: BTreeMap::new(),
        }
    }
}

pub struct CustomCollector {
    cfg: CustomProviderConfig,
    store: UsageStore,
    /// Cumulative totals per session as of the previous scan, for deltas.
    last_totals: BTreeMap<String, TokenCounts>,
    /// Recent percentage observations per window (kind|scope), for burn
    /// trends. Kept bounded.
    quota_trends: BTreeMap<String, Vec<QuotaSample>>,
}

impl CustomCollector {
    pub fn from_config(cfg: &CustomProviderConfig) -> Self {
        CustomCollector {
            cfg: cfg.clone(),
            store: UsageStore::new(Provider::Custom),
            last_totals: BTreeMap::new(),
            quota_trends: BTreeMap::new(),
        }
    }

    pub fn discovery_info(&self) -> crate::diagnostics::DiscoveryInfo {
        crate::diagnostics::DiscoveryInfo {
            provider: Provider::Custom,
            installed: self.cfg.source.as_ref().is_some_and(|p| p.exists())
                || self.cfg.command.is_some(),
            session_dirs: Vec::new(),
            session_files: 0,
            auth_present: false,
            cli_version: None,
        }
    }

    fn fetch(&self) -> Result<String, String> {
        if let Some(path) = &self.cfg.source {
            return std::fs::read_to_string(path).map_err(|e| {
                format!(
                    "reading {}: {e}",
                    crate::diagnostics::redact_path(&path.display().to_string())
                )
            });
        }
        if let Some(command) = &self.cfg.command {
            let out = std::process::Command::new("bash")
                .arg("-c")
                .arg(command)
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| format!("running custom provider command: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "custom provider command exited with {}",
                    out.status
                ));
            }
            return String::from_utf8(out.stdout)
                .map_err(|_| "custom provider command wrote non-UTF-8 output".into());
        }
        Err("custom provider enabled but neither source nor command configured".into())
    }
}

impl Collector for CustomCollector {
    fn provider(&self) -> Provider {
        Provider::Custom
    }

    fn scan(&mut self, ctx: &ScanContext) -> ProviderSnapshot {
        let report: ExternalReport = match self.fetch().and_then(|text| {
            serde_json::from_str(&text).map_err(|e| format!("parsing provider JSON: {e}"))
        }) {
            Ok(r) => r,
            Err(msg) => {
                let mut health = CollectorHealth::unavailable(msg);
                health.status = CollectorStatus::Error;
                health.last_scan = Some(ctx.now);
                return ProviderSnapshot::empty(Provider::Custom, health);
            }
        };
        let captured_at = report.captured_at.unwrap_or(ctx.now);

        // Sessions: cumulative counters -> deltas -> usage events.
        for s in &report.sessions {
            let totals: TokenCounts = (&s.tokens).into();
            let prev = self.last_totals.get(&s.id).cloned().unwrap_or_default();
            let delta = TokenCounts::saturating_delta(&totals, &prev);
            self.last_totals.insert(s.id.clone(), totals);
            let model = s.model.as_deref().map(ModelIdentity::normalize);
            {
                let record = self.store.session_mut(&s.id);
                record.project = s.project.clone();
                record.context_tokens = s.context_tokens;
                record.context_window = s.context_window;
                if record.started_at.is_none() {
                    record.started_at = s.started_at;
                }
                if let Some(t) = s.last_activity
                    && record.last_activity.is_none_or(|prev| t > prev)
                {
                    record.last_activity = Some(t);
                }
                if let Some(m) = &model {
                    record.last_model = Some(m.clone());
                }
            }
            if !delta.is_zero() {
                let at = s.last_activity.unwrap_or(captured_at);
                self.store.record_event(&s.id, at, model, delta);
            }
        }
        self.store.trim(ctx);

        // Quota windows, with the same burn-trend math as built-ins.
        let mut quota_windows = Vec::new();
        for w in &report.quota_windows {
            let kind = QuotaWindowKind::from_window_minutes(w.window_minutes);
            let key = format!("{:?}|{}", kind, w.scope.clone().unwrap_or_default());
            let trend = self.quota_trends.entry(key).or_default();
            if trend
                .last()
                .is_none_or(|s| s.captured_at != captured_at || s.used_percent != w.used_percent)
            {
                trend.push(QuotaSample {
                    captured_at,
                    used_percent: w.used_percent,
                });
                if trend.len() > 200 {
                    trend.drain(..trend.len() - 200);
                }
            }
            let projection = project_quota(trend, ctx.now);
            quota_windows.push(QuotaWindow {
                kind,
                used_percent: w.used_percent,
                window_minutes: w.window_minutes,
                resets_at: w.resets_at,
                captured_at,
                scope: w.scope.clone(),
                burn_per_hour: projection.burn_per_hour,
                projected_exhaustion: projection.projected_exhaustion,
                trend_confidence: projection.confidence,
            });
        }

        let mut capabilities = Vec::new();
        if !report.sessions.is_empty() {
            capabilities.extend([
                Capability::LocalTokenUsage,
                Capability::ActiveSession,
                Capability::CalendarWeekAggregation,
                Capability::ModelBreakdown,
                Capability::History,
            ]);
        }
        if !quota_windows.is_empty() {
            capabilities.push(Capability::ProviderQuota);
            if quota_windows.iter().any(|w| w.resets_at.is_some()) {
                capabilities.push(Capability::ResetTimes);
            }
        }
        if report.credits.is_some() {
            capabilities.push(Capability::Credits);
        }

        let health = CollectorHealth {
            status: CollectorStatus::Ok,
            message: None,
            last_scan: Some(ctx.now),
            files_scanned: 1,
            parse_errors: 0,
        };
        let credits = report.credits.map(|balance| Credits {
            balance,
            captured_at,
        });
        self.store
            .build_snapshot(ctx, capabilities, health, quota_windows, credits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn ctx(now: &str) -> ScanContext {
        let now: DateTime<Utc> = now.parse().unwrap();
        ScanContext {
            now,
            week_start: now - Duration::days(3),
            week_end: now + Duration::days(4),
            history_retention_minutes: 60,
        }
    }

    fn collector_with_file(json: &str, name: &str) -> (CustomCollector, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lmtop-custom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, json).unwrap();
        let cfg = CustomProviderConfig {
            enabled: true,
            name: "Gemini".into(),
            source: Some(path.clone()),
            command: None,
        };
        (CustomCollector::from_config(&cfg), path)
    }

    #[test]
    fn parses_report_and_computes_deltas() {
        let json = r#"{
            "quota_windows": [{"used_percent": 40.0, "window_minutes": 300,
                               "resets_at": "2026-07-17T14:00:00Z"}],
            "credits": 7.5,
            "sessions": [{"id": "s1", "model": "gemini-2.5-pro", "project": "app",
                          "last_activity": "2026-07-17T09:59:00Z",
                          "tokens": {"input": 100, "output": 50}}]
        }"#;
        let (mut c, path) = collector_with_file(json, "report1.json");
        let snap = c.scan(&ctx("2026-07-17T10:00:00Z"));
        assert_eq!(snap.provider, Provider::Custom);
        assert_eq!(snap.health.status, CollectorStatus::Ok);
        assert_eq!(snap.sessions.len(), 1);
        assert_eq!(snap.sessions[0].tokens.total(), 150);
        assert_eq!(snap.quota_windows.len(), 1);
        assert_eq!(snap.quota_windows[0].kind, QuotaWindowKind::FiveHour);
        assert_eq!(snap.credits.as_ref().unwrap().balance, 7.5);
        assert!(snap.supports(Capability::ProviderQuota));
        assert!(snap.supports(Capability::ResetTimes));

        // Second scan with grown counters: only the delta lands in the week.
        std::fs::write(
            &path,
            r#"{"sessions": [{"id": "s1",
                "last_activity": "2026-07-17T10:04:00Z",
                "tokens": {"input": 160, "output": 90}}]}"#,
        )
        .unwrap();
        let snap = c.scan(&ctx("2026-07-17T10:05:00Z"));
        assert_eq!(snap.sessions[0].tokens.total(), 250);
        assert_eq!(snap.week.as_ref().unwrap().tokens.total(), 250);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn shrinking_counters_never_go_negative() {
        let json = r#"{"sessions": [{"id": "s1", "tokens": {"input": 500, "output": 100}}]}"#;
        let (mut c, path) = collector_with_file(json, "report2.json");
        c.scan(&ctx("2026-07-17T10:00:00Z"));
        std::fs::write(
            &path,
            r#"{"sessions": [{"id": "s1", "tokens": {"input": 10, "output": 5}}]}"#,
        )
        .unwrap();
        let snap = c.scan(&ctx("2026-07-17T10:05:00Z"));
        // No panic, no absurd delta; totals stay at the first observation.
        assert_eq!(snap.sessions[0].tokens.total(), 600);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_source_reports_error_health() {
        let cfg = CustomProviderConfig {
            enabled: true,
            name: "X".into(),
            source: Some("/nonexistent/lmtop-custom.json".into()),
            command: None,
        };
        let mut c = CustomCollector::from_config(&cfg);
        let snap = c.scan(&ctx("2026-07-17T10:00:00Z"));
        assert_eq!(snap.health.status, CollectorStatus::Error);
        assert!(snap.sessions.is_empty());
    }

    #[test]
    fn unconfigured_source_is_an_error() {
        let cfg = CustomProviderConfig {
            enabled: true,
            ..Default::default()
        };
        let mut c = CustomCollector::from_config(&cfg);
        let snap = c.scan(&ctx("2026-07-17T10:00:00Z"));
        assert_eq!(snap.health.status, CollectorStatus::Error);
    }
}

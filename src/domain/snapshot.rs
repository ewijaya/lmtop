use super::{Credits, ModelIdentity, Provider, QuotaWindow, SessionUsage, TokenCounts};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A feature a collector may or may not support. The UI renders unsupported
/// capabilities as "unavailable", never as errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    LocalTokenUsage,
    ActiveSession,
    CalendarWeekAggregation,
    ModelBreakdown,
    ProviderQuota,
    Credits,
    ResetTimes,
    History,
}

impl Capability {
    pub const ALL: [Capability; 8] = [
        Capability::LocalTokenUsage,
        Capability::ActiveSession,
        Capability::CalendarWeekAggregation,
        Capability::ModelBreakdown,
        Capability::ProviderQuota,
        Capability::Credits,
        Capability::ResetTimes,
        Capability::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Capability::LocalTokenUsage => "local_token_usage",
            Capability::ActiveSession => "active_session",
            Capability::CalendarWeekAggregation => "calendar_week_aggregation",
            Capability::ModelBreakdown => "model_breakdown",
            Capability::ProviderQuota => "provider_quota",
            Capability::Credits => "credits",
            Capability::ResetTimes => "reset_times",
            Capability::History => "history",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectorStatus {
    Ok,
    Degraded,
    Error,
    Disabled,
    Unavailable,
}

/// Health of one collector, shown in the header and `doctor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorHealth {
    pub status: CollectorStatus,
    /// Human-readable detail. Must already be redacted; never contains
    /// credentials, prompt content, or home-relative secrets.
    pub message: Option<String>,
    pub last_scan: Option<DateTime<Utc>>,
    pub files_scanned: u64,
    pub parse_errors: u64,
}

impl CollectorHealth {
    pub fn unavailable(message: impl Into<String>) -> Self {
        CollectorHealth {
            status: CollectorStatus::Unavailable,
            message: Some(message.into()),
            last_scan: None,
            files_scanned: 0,
            parse_errors: 0,
        }
    }

    pub fn disabled() -> Self {
        CollectorHealth {
            status: CollectorStatus::Disabled,
            message: Some("disabled by configuration".into()),
            last_scan: None,
            files_scanned: 0,
            parse_errors: 0,
        }
    }
}

/// Data freshness classification, derived from the last successful scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Stale,
    Unavailable,
}

impl Freshness {
    pub fn from_last_scan(
        last_scan: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        stale_after_secs: i64,
    ) -> Freshness {
        match last_scan {
            Some(t) if now.signed_duration_since(t).num_seconds() <= stale_after_secs => {
                Freshness::Fresh
            }
            Some(_) => Freshness::Stale,
            None => Freshness::Unavailable,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Freshness::Fresh => "fresh",
            Freshness::Stale => "stale",
            Freshness::Unavailable => "unavailable",
        }
    }
}

/// One time-bucketed sample of observed token throughput, used for the
/// token-rate chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistorySample {
    /// Start of the minute bucket this sample covers.
    pub at: DateTime<Utc>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Aggregated usage for one calendar week (local timezone, configurable
/// week start). Never confuse this with a provider's rolling quota window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeekAggregate {
    /// Inclusive start of the week in UTC.
    pub week_start: DateTime<Utc>,
    /// Exclusive end of the week in UTC.
    pub week_end: DateTime<Utc>,
    pub tokens: TokenCounts,
    /// Per-model split keyed by raw model id.
    pub by_model: BTreeMap<String, ModelWeekUsage>,
    /// Number of sessions that contributed usage this week.
    pub sessions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelWeekUsage {
    pub model: ModelIdentity,
    pub tokens: TokenCounts,
}

/// Everything the application knows about one provider at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub provider: Provider,
    pub capabilities: Vec<Capability>,
    pub health: CollectorHealth,
    /// Sessions with any activity in the recent lookback window,
    /// newest first.
    pub sessions: Vec<SessionUsage>,
    /// Current calendar-week aggregate (observed tokens).
    pub week: Option<WeekAggregate>,
    /// Provider-reported quota windows, if the provider exposes them.
    pub quota_windows: Vec<QuotaWindow>,
    /// Provider-reported credits, if exposed.
    pub credits: Option<Credits>,
    /// Per-minute throughput samples for the recent history window.
    pub history: Vec<HistorySample>,
    /// Tokens observed in the currently active session(s), i.e. sessions
    /// with activity in the last few minutes.
    pub current_session_tokens: TokenCounts,
}

impl ProviderSnapshot {
    pub fn empty(provider: Provider, health: CollectorHealth) -> Self {
        ProviderSnapshot {
            provider,
            capabilities: Vec::new(),
            health,
            sessions: Vec::new(),
            week: None,
            quota_windows: Vec::new(),
            credits: None,
            history: Vec::new(),
            current_session_tokens: TokenCounts::default(),
        }
    }

    pub fn supports(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    pub fn quota_window(&self, kind: &super::QuotaWindowKind) -> Option<&QuotaWindow> {
        self.quota_windows.iter().find(|w| &w.kind == kind)
    }
}

/// The complete normalized state handed to the UI and to `snapshot --json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub generated_at: DateTime<Utc>,
    pub providers: BTreeMap<Provider, ProviderSnapshot>,
}

impl UsageSnapshot {
    pub fn new(generated_at: DateTime<Utc>) -> Self {
        UsageSnapshot {
            generated_at,
            providers: BTreeMap::new(),
        }
    }
}

//! Shared in-memory usage store used by both collectors: accumulates
//! deduplicated usage events plus per-session metadata, and assembles
//! normalized provider snapshots from them.

use super::ScanContext;
use crate::aggregation::{self, UsageEvent};
use crate::domain::{
    Capability, CollectorHealth, Credits, ModelIdentity, Provider, ProviderSnapshot, QuotaWindow,
    SessionState, SessionUsage, TokenCounts,
};
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

use super::SESSION_LOOKBACK_HOURS;

/// Lookback for the tokens/minute column.
const RATE_LOOKBACK_MINUTES: i64 = 5;

#[derive(Debug, Clone, Default)]
pub struct SessionRecord {
    pub project: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub last_activity: Option<DateTime<Utc>>,
    pub last_model: Option<ModelIdentity>,
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub tokens: TokenCounts,
    pub tokens_by_model: BTreeMap<String, TokenCounts>,
}

#[derive(Debug)]
pub struct UsageStore {
    provider: Provider,
    sessions: BTreeMap<String, SessionRecord>,
    events: Vec<UsageEvent>,
}

impl UsageStore {
    pub fn new(provider: Provider) -> Self {
        UsageStore {
            provider,
            sessions: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    pub fn session_mut(&mut self, id: &str) -> &mut SessionRecord {
        self.sessions.entry(id.to_string()).or_default()
    }

    /// Record one deduplicated usage delta. Updates both the event stream
    /// (for week/history aggregation) and the session accumulators.
    pub fn record_event(
        &mut self,
        session_id: &str,
        timestamp: DateTime<Utc>,
        model: Option<ModelIdentity>,
        tokens: TokenCounts,
    ) {
        let record = self.sessions.entry(session_id.to_string()).or_default();
        record.tokens.add(&tokens);
        if let Some(m) = &model {
            record
                .tokens_by_model
                .entry(m.raw.clone())
                .or_default()
                .add(&tokens);
            record.last_model = Some(m.clone());
        }
        if record.last_activity.is_none_or(|t| timestamp > t) {
            record.last_activity = Some(timestamp);
        }
        if record.started_at.is_none_or(|t| timestamp < t) {
            record.started_at = Some(timestamp);
        }
        self.events.push(UsageEvent {
            session_id: session_id.to_string(),
            timestamp,
            model,
            tokens,
        });
    }

    /// Drop events older than the retention cutoff. Session accumulators are
    /// kept (session totals are all-time within the scan horizon).
    pub fn trim(&mut self, ctx: &ScanContext) {
        let cutoff = ctx.retention_cutoff();
        self.events.retain(|e| e.timestamp >= cutoff);
    }

    /// Assemble a normalized snapshot from everything recorded so far.
    #[allow(clippy::too_many_arguments)]
    pub fn build_snapshot(
        &self,
        ctx: &ScanContext,
        capabilities: Vec<Capability>,
        health: CollectorHealth,
        quota_windows: Vec<QuotaWindow>,
        credits: Option<Credits>,
    ) -> ProviderSnapshot {
        let lookback = ctx.now - Duration::hours(SESSION_LOOKBACK_HOURS);
        let mut sessions: Vec<SessionUsage> = self
            .sessions
            .iter()
            .filter(|(_, r)| r.last_activity.is_some_and(|t| t >= lookback))
            .map(|(id, r)| SessionUsage {
                provider: self.provider,
                id: id.clone(),
                model: r.last_model.clone(),
                project: r.project.clone(),
                started_at: r.started_at,
                last_activity: r.last_activity,
                tokens: r.tokens.clone(),
                tokens_by_model: r.tokens_by_model.clone(),
                context_tokens: r.context_tokens,
                context_window: r.context_window,
                tokens_per_minute: aggregation::tokens_per_minute(
                    &self.events,
                    id,
                    ctx.now,
                    RATE_LOOKBACK_MINUTES,
                ),
            })
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));

        let mut current_session_tokens = TokenCounts::default();
        for s in &sessions {
            if s.state(ctx.now) == SessionState::Active {
                current_session_tokens.add(&s.tokens);
            }
        }

        let week = aggregation::build_week(&self.events, ctx.week_start, ctx.week_end);
        let history =
            aggregation::build_history(&self.events, ctx.now, ctx.history_retention_minutes);

        ProviderSnapshot {
            provider: self.provider,
            capabilities,
            health,
            sessions,
            week: Some(week),
            quota_windows,
            credits,
            history,
            current_session_tokens,
        }
    }
}

/// Basename of a working directory for display, without exposing the full
/// filesystem path.
pub fn project_name(cwd: &str) -> Option<String> {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    let name = trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())?;
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_takes_basename() {
        assert_eq!(
            project_name("/home/u/projects/agentop"),
            Some("agentop".into())
        );
        assert_eq!(project_name("C:\\Users\\u\\code\\app"), Some("app".into()));
        assert_eq!(project_name("/"), None);
    }
}

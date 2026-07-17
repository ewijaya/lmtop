//! Provider collectors. Each collector owns discovery, incremental parsing,
//! deduplication, and cumulative-to-delta conversion for one provider, and
//! produces a normalized [`ProviderSnapshot`]. Nothing outside this module
//! touches provider-specific schemas.

pub mod claude;
pub mod codex;
mod jsonl;
pub mod live_quota;
mod store;

use crate::domain::{Provider, ProviderSnapshot};
use chrono::{DateTime, Utc};

/// Everything a collector needs to know about "now" for one scan pass.
/// Time is always injected so tests can use deterministic clocks.
#[derive(Debug, Clone)]
pub struct ScanContext {
    pub now: DateTime<Utc>,
    pub week_start: DateTime<Utc>,
    pub week_end: DateTime<Utc>,
    pub history_retention_minutes: u64,
}

/// How far back the session table looks; scans must cover at least this
/// horizon even right after a week boundary.
pub const SESSION_LOOKBACK_HOURS: i64 = 48;

impl ScanContext {
    /// Oldest timestamp that can still contribute to any view: the current
    /// week (weekly aggregate), the history window (rate chart), or the
    /// session lookback (session table).
    pub fn retention_cutoff(&self) -> DateTime<Utc> {
        let history_cutoff =
            self.now - chrono::Duration::minutes(self.history_retention_minutes as i64);
        let session_cutoff = self.now - chrono::Duration::hours(SESSION_LOOKBACK_HOURS);
        self.week_start.min(history_cutoff).min(session_cutoff)
    }
}

/// A usage collector for one provider. `scan` is synchronous and file-bound;
/// the runtime calls it from a blocking task at the configured refresh
/// interval, never at render frequency.
pub trait Collector: Send {
    fn provider(&self) -> Provider;
    fn scan(&mut self, ctx: &ScanContext) -> ProviderSnapshot;
}

/// Directory candidates that actually exist on this machine, given a set of
/// default locations plus configured extras.
pub fn existing_dirs(
    defaults: &[std::path::PathBuf],
    extra: &[std::path::PathBuf],
) -> Vec<std::path::PathBuf> {
    defaults
        .iter()
        .chain(extra.iter())
        .filter(|p| p.is_dir())
        .cloned()
        .collect()
}

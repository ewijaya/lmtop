use super::{ModelIdentity, Provider, TokenCounts};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Liveness of a session as inferred from its last activity timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Recent,
    Idle,
}

/// Normalized usage metadata for one agent session. Contains no prompt
/// bodies, tool output, or other conversation content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUsage {
    pub provider: Provider,
    /// Provider session identifier (UUID-like).
    pub id: String,
    /// Last model observed in the session.
    pub model: Option<ModelIdentity>,
    /// Project directory name (basename of the session cwd), for display.
    pub project: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub last_activity: Option<DateTime<Utc>>,
    /// Total observed tokens for this session.
    pub tokens: TokenCounts,
    /// Per-model split of `tokens`, keyed by raw model id.
    #[serde(default)]
    pub tokens_by_model: BTreeMap<String, TokenCounts>,
    /// Tokens currently occupying the model context window, when known.
    pub context_tokens: Option<u64>,
    /// Size of the model context window, when known.
    pub context_window: Option<u64>,
    /// Observed tokens per minute over the recent activity window.
    pub tokens_per_minute: Option<f64>,
}

impl SessionUsage {
    pub fn state(&self, now: DateTime<Utc>) -> SessionState {
        match self.last_activity {
            Some(t) if now.signed_duration_since(t) <= chrono::Duration::minutes(5) => {
                SessionState::Active
            }
            Some(t) if now.signed_duration_since(t) <= chrono::Duration::hours(2) => {
                SessionState::Recent
            }
            _ => SessionState::Idle,
        }
    }

    /// Context window utilization in percent, when both sides are known.
    pub fn context_percent(&self) -> Option<f64> {
        match (self.context_tokens, self.context_window) {
            (Some(used), Some(win)) if win > 0 => Some(used as f64 / win as f64 * 100.0),
            _ => None,
        }
    }
}

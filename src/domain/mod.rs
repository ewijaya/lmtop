//! Normalized domain types shared by collectors, aggregation, and the UI.
//!
//! The UI must never depend on provider-specific schemas; everything a
//! collector learns is translated into these types first.

mod model;
mod quota;
mod session;
mod snapshot;
mod tokens;

pub use model::{ModelFamily, ModelIdentity};
pub use quota::{Credits, QuotaOutlook, QuotaWindow, QuotaWindowKind};
pub use session::{SessionState, SessionUsage};
pub use snapshot::{
    Capability, CollectorHealth, CollectorStatus, Freshness, HistorySample, ModelWeekUsage,
    ProviderSnapshot, UsageSnapshot, WeekAggregate,
};
pub use tokens::TokenCounts;

use serde::{Deserialize, Serialize};

/// An AI coding agent product whose usage we monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Codex, Provider::Claude];

    pub fn display_name(self) -> &'static str {
        match self {
            Provider::Codex => "Codex",
            Provider::Claude => "Claude",
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

impl std::str::FromStr for Provider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "codex" => Ok(Provider::Codex),
            "claude" => Ok(Provider::Claude),
            other => Err(format!("unknown provider: {other}")),
        }
    }
}

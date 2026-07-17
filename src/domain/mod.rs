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
pub use quota::{Credits, QuotaOutlook, QuotaWindow, QuotaWindowKind, TrendConfidence};
pub use session::{SessionState, SessionUsage};
pub use snapshot::{
    Capability, CollectorHealth, CollectorStatus, Freshness, HistorySample, ModelWeekUsage,
    ProviderSnapshot, UsageSnapshot, WeekAggregate,
};
pub use tokens::TokenCounts;

use serde::{Deserialize, Serialize};

/// An AI coding agent product whose usage we monitor. `Custom` is a
/// user-defined provider fed by the external-source collector (a JSON file
/// or command configured in `[providers.custom]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
    Custom,
}

/// Display label for the custom provider, set once at startup from the
/// config (`providers.custom.name`). Leaked so `display_name` can stay
/// `&'static str` everywhere.
static CUSTOM_LABEL: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

pub fn set_custom_provider_label(name: &str) {
    let name = name.trim();
    if !name.is_empty() {
        let _ = CUSTOM_LABEL.set(Box::leak(name.to_string().into_boxed_str()));
    }
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Codex, Provider::Claude, Provider::Custom];

    pub fn display_name(self) -> &'static str {
        match self {
            Provider::Codex => "Codex",
            Provider::Claude => "Claude",
            Provider::Custom => CUSTOM_LABEL.get().copied().unwrap_or("Custom"),
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
            "custom" => Ok(Provider::Custom),
            other => Err(format!("unknown provider: {other}")),
        }
    }
}

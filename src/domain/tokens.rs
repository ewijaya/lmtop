use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Token counts observed from local session metadata.
///
/// Semantics (applied identically to every provider):
/// - `input`: non-cached input tokens actually processed.
/// - `cached_input`: input tokens served from the provider's prompt cache.
/// - `cache_creation`: input tokens written into the prompt cache
///   (reported by Claude; zero for providers that do not expose it).
/// - `output`: all generated tokens, including reasoning tokens.
/// - `reasoning`: the subset of `output` spent on reasoning, when reported.
/// - `other`: unknown token categories preserved for forward compatibility.
///
/// The displayed total is `input + cached_input + cache_creation + output`,
/// i.e. cached input IS included in totals. `reasoning` is informational and
/// never added on top of `output`. `other` categories are tracked but not
/// folded into the total because their semantics are unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCounts {
    pub input: u64,
    pub cached_input: u64,
    pub cache_creation: u64,
    pub output: u64,
    pub reasoning: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, u64>,
}

impl TokenCounts {
    /// Total tokens as displayed: non-cached input + cached input +
    /// cache-creation input + output.
    pub fn total(&self) -> u64 {
        self.input + self.cached_input + self.cache_creation + self.output
    }

    /// All input-side tokens (non-cached + cached + cache creation).
    pub fn total_input(&self) -> u64 {
        self.input + self.cached_input + self.cache_creation
    }

    pub fn is_zero(&self) -> bool {
        self.total() == 0 && self.other.values().all(|v| *v == 0)
    }

    pub fn add(&mut self, other: &TokenCounts) {
        self.input += other.input;
        self.cached_input += other.cached_input;
        self.cache_creation += other.cache_creation;
        self.output += other.output;
        self.reasoning += other.reasoning;
        for (k, v) in &other.other {
            *self.other.entry(k.clone()).or_insert(0) += v;
        }
    }

    /// Delta between two cumulative counters (`newer - older`), saturating at
    /// zero per category so a provider-side counter reset never produces
    /// negative or absurd deltas.
    pub fn saturating_delta(newer: &TokenCounts, older: &TokenCounts) -> TokenCounts {
        let mut other = BTreeMap::new();
        for (k, v) in &newer.other {
            let prev = older.other.get(k).copied().unwrap_or(0);
            let d = v.saturating_sub(prev);
            if d > 0 {
                other.insert(k.clone(), d);
            }
        }
        TokenCounts {
            input: newer.input.saturating_sub(older.input),
            cached_input: newer.cached_input.saturating_sub(older.cached_input),
            cache_creation: newer.cache_creation.saturating_sub(older.cache_creation),
            output: newer.output.saturating_sub(older.output),
            reasoning: newer.reasoning.saturating_sub(older.reasoning),
            other,
        }
    }
}

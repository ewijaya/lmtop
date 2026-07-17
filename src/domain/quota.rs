use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Stable classification of a provider-defined rolling quota window.
///
/// Classification is done by window duration, never by array position:
/// ~300 minutes is the five-hour window, ~10080 minutes the weekly window.
/// Anything else stays `Unknown` and is displayed as such.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaWindowKind {
    FiveHour,
    Weekly,
    Unknown,
}

impl QuotaWindowKind {
    /// Classify by duration with generous tolerance. `None` durations are
    /// unknown by definition.
    pub fn from_window_minutes(minutes: Option<u64>) -> QuotaWindowKind {
        match minutes {
            Some(m) if (240..=360).contains(&m) => QuotaWindowKind::FiveHour,
            Some(m) if (9_000..=11_500).contains(&m) => QuotaWindowKind::Weekly,
            _ => QuotaWindowKind::Unknown,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            QuotaWindowKind::FiveHour => "5h",
            QuotaWindowKind::Weekly => "Weekly",
            QuotaWindowKind::Unknown => "Window",
        }
    }
}

/// A provider-reported subscription quota window. This is authoritative
/// provider data (a percentage), never derived from locally observed tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub kind: QuotaWindowKind,
    /// Percentage of the window's quota already used, 0.0..=100.0,
    /// exactly as reported by the provider.
    pub used_percent: f64,
    /// Window length in minutes, when reported.
    pub window_minutes: Option<u64>,
    /// When the window resets, when reported.
    pub resets_at: Option<DateTime<Utc>>,
    /// When this snapshot was captured from provider data.
    pub captured_at: DateTime<Utc>,
    /// Scope qualifier for model-specific limits (e.g. `Fable` for a
    /// per-model weekly cap), when the provider reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Burn velocity in percentage points per hour, estimated from the
    /// recent trend of provider-reported percentages. `None` when there are
    /// not enough samples.
    pub burn_per_hour: Option<f64>,
    /// Projected instant the window reaches 100%, extrapolating
    /// `burn_per_hour` from the latest report. `None` when the burn rate is
    /// unknown or (effectively) zero.
    pub projected_exhaustion: Option<DateTime<Utc>>,
    /// Confidence grade of the burn trend, present whenever
    /// `burn_per_hour` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trend_confidence: Option<TrendConfidence>,
}

/// How much to trust a burn-velocity trend, graded from how many samples
/// the current monotonic run has, how long it spans, and how fresh its
/// newest sample is. Displayed alongside every projection so estimates are
/// never mistaken for provider-reported facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendConfidence {
    Low,
    Medium,
    High,
}

impl TrendConfidence {
    /// Grade a trend: High needs a well-populated, long, fresh run;
    /// Medium a moderate one; anything that met the minimum bar is Low.
    pub fn grade(run_samples: usize, span_minutes: f64, age_minutes: i64) -> TrendConfidence {
        if run_samples >= 5 && span_minutes >= 30.0 && age_minutes <= 10 {
            TrendConfidence::High
        } else if run_samples >= 3 && span_minutes >= 10.0 && age_minutes <= 20 {
            TrendConfidence::Medium
        } else {
            TrendConfidence::Low
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TrendConfidence::Low => "low",
            TrendConfidence::Medium => "medium",
            TrendConfidence::High => "high",
        }
    }

    /// Compact form for tight TUI rows.
    pub fn short_label(self) -> &'static str {
        match self {
            TrendConfidence::Low => "low",
            TrendConfidence::Medium => "med",
            TrendConfidence::High => "high",
        }
    }
}

/// Capacity-planning verdict: will this window run out before it resets?
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict")]
pub enum QuotaOutlook {
    /// Already at (or beyond) the limit.
    Exhausted,
    /// Projected to hit 100% before the reset.
    AtRisk { projected_exhaustion: DateTime<Utc> },
    /// Projected to last until the reset at the current burn rate.
    Lasts,
    /// Not enough information (no burn trend, or no reset time to compare
    /// against a projection).
    Unknown,
}

impl QuotaWindow {
    /// Human label, e.g. "5h" / "Weekly" / "Weekly (Fable)" /
    /// "Window (90m)".
    pub fn label(&self) -> String {
        let base = match (&self.kind, self.window_minutes) {
            (QuotaWindowKind::Unknown, Some(m)) => format!("Window ({m}m)"),
            (kind, _) => kind.label().to_string(),
        };
        match &self.scope {
            Some(scope) => format!("{base} ({scope})"),
            None => base,
        }
    }

    /// Whether the window is projected to run out before its reset. Uses
    /// only provider-reported percentages and their trend — never locally
    /// observed token counts.
    pub fn outlook(&self) -> QuotaOutlook {
        if self.used_percent >= 100.0 {
            return QuotaOutlook::Exhausted;
        }
        match (self.projected_exhaustion, self.resets_at) {
            (Some(exhaustion), Some(reset)) => {
                if exhaustion <= reset {
                    QuotaOutlook::AtRisk {
                        projected_exhaustion: exhaustion,
                    }
                } else {
                    QuotaOutlook::Lasts
                }
            }
            // Burning but no known reset: still worth surfacing the ETA.
            (Some(exhaustion), None) => QuotaOutlook::AtRisk {
                projected_exhaustion: exhaustion,
            },
            // No measurable burn: lasts to the reset if we know one.
            (None, Some(_)) => {
                if self.burn_per_hour.is_some() {
                    QuotaOutlook::Lasts
                } else {
                    QuotaOutlook::Unknown
                }
            }
            (None, None) => QuotaOutlook::Unknown,
        }
    }
}

/// Provider-reported credit balance (e.g. Codex flex credits).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Credits {
    pub balance: f64,
    pub captured_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_five_hour_window() {
        assert_eq!(
            QuotaWindowKind::from_window_minutes(Some(300)),
            QuotaWindowKind::FiveHour
        );
    }

    #[test]
    fn classifies_weekly_window() {
        assert_eq!(
            QuotaWindowKind::from_window_minutes(Some(10_080)),
            QuotaWindowKind::Weekly
        );
    }

    fn window(
        used: f64,
        exhaustion: Option<&str>,
        reset: Option<&str>,
        burn: Option<f64>,
    ) -> QuotaWindow {
        QuotaWindow {
            kind: QuotaWindowKind::FiveHour,
            used_percent: used,
            window_minutes: Some(300),
            resets_at: reset.map(|s| s.parse().unwrap()),
            captured_at: "2026-07-17T10:00:00Z".parse().unwrap(),
            scope: None,
            burn_per_hour: burn,
            projected_exhaustion: exhaustion.map(|s| s.parse().unwrap()),
            trend_confidence: burn.map(|_| TrendConfidence::Medium),
        }
    }

    #[test]
    fn outlook_at_risk_when_exhaustion_before_reset() {
        let w = window(
            80.0,
            Some("2026-07-17T12:00:00Z"),
            Some("2026-07-17T14:00:00Z"),
            Some(10.0),
        );
        assert!(matches!(w.outlook(), QuotaOutlook::AtRisk { .. }));
    }

    #[test]
    fn outlook_lasts_when_reset_comes_first() {
        let w = window(
            40.0,
            Some("2026-07-17T20:00:00Z"),
            Some("2026-07-17T14:00:00Z"),
            Some(5.0),
        );
        assert_eq!(w.outlook(), QuotaOutlook::Lasts);
    }

    #[test]
    fn outlook_exhausted_at_100() {
        let w = window(100.0, None, Some("2026-07-17T14:00:00Z"), None);
        assert_eq!(w.outlook(), QuotaOutlook::Exhausted);
    }

    #[test]
    fn outlook_unknown_without_data() {
        let w = window(50.0, None, None, None);
        assert_eq!(w.outlook(), QuotaOutlook::Unknown);
    }

    #[test]
    fn zero_burn_with_reset_lasts() {
        let w = window(50.0, None, Some("2026-07-17T14:00:00Z"), Some(0.0));
        assert_eq!(w.outlook(), QuotaOutlook::Lasts);
    }

    #[test]
    fn confidence_grading() {
        assert_eq!(TrendConfidence::grade(6, 45.0, 5), TrendConfidence::High);
        assert_eq!(TrendConfidence::grade(3, 12.0, 15), TrendConfidence::Medium);
        assert_eq!(TrendConfidence::grade(2, 5.0, 25), TrendConfidence::Low);
        // Many samples but stale -> not High.
        assert_eq!(TrendConfidence::grade(9, 60.0, 25), TrendConfidence::Low);
    }

    #[test]
    fn unknown_durations_stay_unknown() {
        assert_eq!(
            QuotaWindowKind::from_window_minutes(Some(60)),
            QuotaWindowKind::Unknown
        );
        assert_eq!(
            QuotaWindowKind::from_window_minutes(None),
            QuotaWindowKind::Unknown
        );
    }
}

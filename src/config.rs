//! Configuration loading. A missing config file is normal: every field has
//! a sensible default so the application works with zero configuration.

use chrono::Weekday;
use color_eyre::eyre::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub providers: ProvidersConfig,
    pub ui: UiConfig,
    pub time: TimeConfig,
    pub history: HistoryConfig,
    pub alerts: AlertsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProvidersConfig {
    pub codex: ProviderConfig,
    pub claude: ProviderConfig,
    /// A user-defined provider fed by an external JSON source; see
    /// `docs/configuration.md` for the schema.
    pub custom: CustomProviderConfig,
}

/// A provider lmtop has no built-in collector for (Gemini, Ollama,
/// OpenRouter, …), fed by a JSON file or command output conforming to the
/// documented external-provider schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CustomProviderConfig {
    /// Off by default; enabling requires a `source` or `command`.
    pub enabled: bool,
    /// Display name shown in the UI (panel titles, session rows).
    pub name: String,
    /// Path to a JSON file conforming to the external-provider schema.
    pub source: Option<PathBuf>,
    /// Command executed each refresh; must print the schema JSON on stdout.
    /// Ignored when `source` is set.
    pub command: Option<String>,
}

impl Default for CustomProviderConfig {
    fn default() -> Self {
        CustomProviderConfig {
            enabled: false,
            name: "Custom".into(),
            source: None,
            command: None,
        }
    }
}

/// Quota alert thresholds and delivery channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AlertsConfig {
    pub enabled: bool,
    /// Fire when a window's used percentage crosses each threshold.
    pub quota_thresholds: Vec<f64>,
    /// Fire when projected exhaustion is within this many minutes and
    /// before the window's reset.
    pub exhaustion_warn_minutes: u64,
    /// Ring the terminal bell.
    pub bell: bool,
    /// Send a desktop notification (notify-send / osascript), best effort.
    pub desktop: bool,
    /// Optional command run on every alert with LMTOP_ALERT_* env vars set.
    pub command: Option<String>,
}

impl Default for AlertsConfig {
    fn default() -> Self {
        AlertsConfig {
            enabled: true,
            quota_thresholds: vec![80.0, 95.0],
            exhaustion_warn_minutes: 30,
            bell: true,
            desktop: true,
            command: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    pub enabled: bool,
    /// Extra directories to search for session files, in addition to the
    /// provider's default location.
    pub session_paths: Vec<PathBuf>,
    /// Allow this provider's optional network-backed quota collector.
    /// Off by default; the application is local-first.
    pub network_quota: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        ProviderConfig {
            enabled: true,
            session_paths: Vec::new(),
            network_quota: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Seconds between collector rescans (not the render rate).
    pub refresh_secs: u64,
    pub theme: String,
    /// Chart drawing symbol: "braille" (highest resolution), "block"
    /// (medium, broader font compatibility), or "tty" (lowest, maximum
    /// terminal compatibility). Unknown values fall back to braille.
    pub graph_symbol: String,
    /// Color depth: "auto" (detect from COLORTERM/TERM), "truecolor",
    /// "256", or "16". SSH does not forward COLORTERM, so a truecolor
    /// terminal over SSH detects as 256 — set "truecolor" to force it.
    pub color_depth: String,
    /// Force ASCII-only gauges and charts.
    pub ascii: bool,
    /// Skip anything that would touch the network.
    pub offline: bool,
    /// Lower redraw rate for reduced motion.
    pub reduced_motion: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            refresh_secs: 5,
            theme: "dark".into(),
            graph_symbol: "braille".into(),
            color_depth: "auto".into(),
            ascii: false,
            offline: false,
            reduced_motion: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimeConfig {
    /// First day of the calendar week: "monday" .. "sunday".
    pub week_start: String,
    /// "local" or a fixed UTC offset in hours (e.g. "+9", "-5").
    pub timezone: String,
}

impl Default for TimeConfig {
    fn default() -> Self {
        TimeConfig {
            week_start: "monday".into(),
            timezone: "local".into(),
        }
    }
}

impl TimeConfig {
    pub fn week_start_day(&self) -> Weekday {
        match self.week_start.to_ascii_lowercase().as_str() {
            "tuesday" => Weekday::Tue,
            "wednesday" => Weekday::Wed,
            "thursday" => Weekday::Thu,
            "friday" => Weekday::Fri,
            "saturday" => Weekday::Sat,
            "sunday" => Weekday::Sun,
            _ => Weekday::Mon,
        }
    }

    /// Fixed offset override in hours, if configured; `None` means local.
    pub fn fixed_offset_hours(&self) -> Option<i32> {
        let tz = self.timezone.trim();
        if tz.eq_ignore_ascii_case("local") || tz.is_empty() {
            None
        } else {
            tz.parse::<i32>().ok().filter(|h| (-14..=14).contains(h))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryConfig {
    /// Minutes of token-rate history kept for the live chart window.
    pub retention_minutes: u64,
    /// Persist rate and quota history across runs (JSONL in the data dir),
    /// powering the history review mode and quota timeline.
    pub persist: bool,
    /// Days of persisted history kept; older entries are pruned at startup.
    pub retention_days: u64,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        HistoryConfig {
            retention_minutes: 60,
            persist: true,
            retention_days: 30,
        }
    }
}

impl Config {
    /// Platform config file location, e.g.
    /// `~/.config/<app>/config.toml` on Linux.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", crate::branding::APP_DIR)
            .map(|d| d.config_dir().join("config.toml"))
    }

    pub fn load(path: Option<&Path>) -> Result<(Config, Option<PathBuf>)> {
        let path = match path {
            Some(p) => Some(p.to_path_buf()),
            None => Self::default_path(),
        };
        match &path {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(p)
                    .wrap_err_with(|| format!("reading config {}", p.display()))?;
                let config: Config = toml::from_str(&text)
                    .wrap_err_with(|| format!("parsing config {}", p.display()))?;
                Ok((config, path))
            }
            _ => Ok((Config::default(), path)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let c = Config::default();
        assert!(c.providers.codex.enabled);
        assert_eq!(c.ui.refresh_secs, 5);
        assert_eq!(c.ui.graph_symbol, "braille");
        assert_eq!(c.time.week_start_day(), Weekday::Mon);
        assert_eq!(c.time.fixed_offset_hours(), None);
    }

    #[test]
    fn parses_partial_config() {
        let c: Config = toml::from_str(
            r#"
            [ui]
            refresh_secs = 10
            ascii = true

            [time]
            week_start = "sunday"
            timezone = "+9"
            "#,
        )
        .unwrap();
        assert_eq!(c.ui.refresh_secs, 10);
        assert!(c.ui.ascii);
        assert_eq!(c.time.week_start_day(), Weekday::Sun);
        assert_eq!(c.time.fixed_offset_hours(), Some(9));
        assert!(c.providers.claude.enabled);
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(toml::from_str::<Config>("[ui]\nbogus = 1\n").is_err());
    }

    /// The "Full reference (defaults shown)" block in docs/configuration.md
    /// is hand-maintained against this file, and nothing else checks that
    /// the two agree. Parse it as a real config: `deny_unknown_fields`
    /// turns a renamed or deleted key into a failure here, and the
    /// assertions below catch a default that drifted in the prose.
    #[test]
    fn documented_reference_config_matches_the_code() {
        let doc = include_str!("../docs/configuration.md");
        let block = doc
            .split("```toml")
            .nth(1)
            .and_then(|s| s.split("```").next())
            .expect("docs/configuration.md has a ```toml reference block");

        // Commented-out optional keys (source, command, alert hook) are
        // illustrative; the live keys must still parse as a whole.
        let parsed: Config = toml::from_str(block).unwrap_or_else(|e| {
            panic!("docs/configuration.md reference block no longer parses: {e}")
        });

        // The block claims to show defaults, so it must round-trip to them.
        let d = Config::default();
        assert_eq!(parsed.ui.refresh_secs, d.ui.refresh_secs);
        assert_eq!(parsed.ui.theme, d.ui.theme);
        assert_eq!(parsed.ui.graph_symbol, d.ui.graph_symbol);
        assert_eq!(parsed.ui.color_depth, d.ui.color_depth);
        assert_eq!(parsed.ui.ascii, d.ui.ascii);
        assert_eq!(parsed.ui.offline, d.ui.offline);
        assert_eq!(parsed.ui.reduced_motion, d.ui.reduced_motion);
        assert_eq!(parsed.providers.codex.enabled, d.providers.codex.enabled);
        assert_eq!(
            parsed.providers.codex.network_quota,
            d.providers.codex.network_quota
        );
        assert_eq!(parsed.providers.claude.enabled, d.providers.claude.enabled);
        assert_eq!(parsed.providers.custom.enabled, d.providers.custom.enabled);
        assert_eq!(parsed.providers.custom.name, d.providers.custom.name);
        assert_eq!(parsed.time.week_start_day(), d.time.week_start_day());
        assert_eq!(
            parsed.time.fixed_offset_hours(),
            d.time.fixed_offset_hours()
        );
        assert_eq!(
            parsed.history.retention_minutes,
            d.history.retention_minutes
        );
        assert_eq!(parsed.history.persist, d.history.persist);
        assert_eq!(parsed.history.retention_days, d.history.retention_days);
        assert_eq!(parsed.alerts.enabled, d.alerts.enabled);
        assert_eq!(parsed.alerts.quota_thresholds, d.alerts.quota_thresholds);
        assert_eq!(
            parsed.alerts.exhaustion_warn_minutes,
            d.alerts.exhaustion_warn_minutes
        );
        assert_eq!(parsed.alerts.bell, d.alerts.bell);
        assert_eq!(parsed.alerts.desktop, d.alerts.desktop);
    }
}

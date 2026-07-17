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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProvidersConfig {
    pub codex: ProviderConfig,
    pub claude: ProviderConfig,
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
    /// Minutes of token-rate history kept for the chart.
    pub retention_minutes: u64,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        HistoryConfig {
            retention_minutes: 60,
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
}

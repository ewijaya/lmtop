//! Dark default theme with distinct provider colors, plus formatting
//! helpers. Falls back to the 16-color ANSI palette when the terminal does
//! not advertise truecolor support.

use crate::domain::{CollectorStatus, Freshness, ModelFamily, Provider};
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub truecolor: bool,
    pub ascii: bool,
}

impl Theme {
    pub fn new(ascii: bool) -> Self {
        let truecolor = std::env::var("COLORTERM")
            .map(|v| v.contains("truecolor") || v.contains("24bit"))
            .unwrap_or(false);
        Theme { truecolor, ascii }
    }

    fn rgb(&self, r: u8, g: u8, b: u8, fallback: Color) -> Color {
        if self.truecolor {
            Color::Rgb(r, g, b)
        } else {
            fallback
        }
    }

    pub fn provider(&self, provider: Provider) -> Color {
        match provider {
            Provider::Codex => self.rgb(122, 195, 255, Color::Cyan),
            Provider::Claude => self.rgb(255, 166, 87, Color::Yellow),
        }
    }

    pub fn provider_dim(&self, provider: Provider) -> Color {
        match provider {
            Provider::Codex => self.rgb(56, 108, 151, Color::Blue),
            Provider::Claude => self.rgb(148, 94, 47, Color::DarkGray),
        }
    }

    pub fn model(&self, family: ModelFamily) -> Color {
        match family {
            ModelFamily::ClaudeFable => self.rgb(214, 143, 255, Color::Magenta),
            ModelFamily::ClaudeOpus => self.rgb(255, 166, 87, Color::Yellow),
            ModelFamily::ClaudeSonnet => self.rgb(255, 209, 128, Color::LightYellow),
            ModelFamily::ClaudeHaiku => self.rgb(170, 219, 160, Color::Green),
            ModelFamily::Gpt => self.rgb(122, 195, 255, Color::Cyan),
            ModelFamily::Other => self.rgb(160, 160, 170, Color::Gray),
        }
    }

    pub fn text(&self) -> Style {
        Style::default().fg(self.rgb(214, 216, 222, Color::White))
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.rgb(120, 124, 138, Color::DarkGray))
    }

    pub fn title(&self) -> Style {
        Style::default()
            .fg(self.rgb(214, 216, 222, Color::White))
            .add_modifier(Modifier::BOLD)
    }

    pub fn border(&self) -> Style {
        Style::default().fg(self.rgb(70, 74, 90, Color::DarkGray))
    }

    pub fn border_focused(&self) -> Style {
        Style::default().fg(self.rgb(140, 150, 190, Color::Cyan))
    }

    pub fn gauge_color(&self, used_percent: f64) -> Color {
        if used_percent >= 90.0 {
            self.rgb(240, 105, 105, Color::Red)
        } else if used_percent >= 70.0 {
            self.rgb(240, 190, 100, Color::Yellow)
        } else {
            self.rgb(120, 200, 140, Color::Green)
        }
    }

    pub fn status(&self, status: CollectorStatus) -> (Color, &'static str) {
        match status {
            CollectorStatus::Ok => (self.rgb(120, 200, 140, Color::Green), "ok"),
            CollectorStatus::Degraded => (self.rgb(240, 190, 100, Color::Yellow), "degraded"),
            CollectorStatus::Error => (self.rgb(240, 105, 105, Color::Red), "error"),
            CollectorStatus::Disabled => (self.rgb(120, 124, 138, Color::DarkGray), "disabled"),
            CollectorStatus::Unavailable => {
                (self.rgb(120, 124, 138, Color::DarkGray), "unavailable")
            }
        }
    }

    pub fn freshness(&self, freshness: Freshness) -> Color {
        match freshness {
            Freshness::Fresh => self.rgb(120, 200, 140, Color::Green),
            Freshness::Stale => self.rgb(240, 190, 100, Color::Yellow),
            Freshness::Unavailable => self.rgb(120, 124, 138, Color::DarkGray),
        }
    }
}

/// Compact human token count: 999, 12.3k, 4.56M, 1.2B.
pub fn fmt_tokens(n: u64) -> String {
    let n = n as f64;
    if n < 1_000.0 {
        format!("{n:.0}")
    } else if n < 1_000_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else if n < 1_000_000_000.0 {
        format!("{:.2}M", n / 1_000_000.0)
    } else {
        format!("{:.2}B", n / 1_000_000_000.0)
    }
}

/// Compact age like "3m", "2h", "5d".
pub fn fmt_age(seconds: i64) -> String {
    if seconds < 0 {
        return "now".into();
    }
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

/// Duration until a reset, like "2h05m" or "3d 4h".
pub fn fmt_duration_until(seconds: i64) -> String {
    if seconds <= 0 {
        return "now".into();
    }
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3600;
    let minutes = (seconds % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_token_counts() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(12_345), "12.3k");
        assert_eq!(fmt_tokens(4_560_000), "4.56M");
        assert_eq!(fmt_tokens(1_200_000_000), "1.20B");
    }

    #[test]
    fn formats_ages_and_durations() {
        assert_eq!(fmt_age(30), "30s");
        assert_eq!(fmt_age(120), "2m");
        assert_eq!(fmt_age(7200), "2h");
        assert_eq!(fmt_age(200_000), "2d");
        assert_eq!(fmt_duration_until(7500), "2h05m");
        assert_eq!(fmt_duration_until(300_000), "3d 11h");
        assert_eq!(fmt_duration_until(-5), "now");
    }
}

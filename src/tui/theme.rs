//! Named color palettes plus formatting helpers. The palette is chosen by
//! `ui.theme` in the config ("dark", "light", "catppuccin", "gruvbox",
//! "nord"); unknown names fall back to dark. Every color degrades to the
//! 16-color ANSI palette when the terminal does not advertise truecolor.

use crate::domain::{CollectorStatus, Freshness, ModelFamily, Provider};
use ratatui::style::{Color, Modifier, Style};

/// A truecolor triple.
type Rgb = (u8, u8, u8);

/// Every color role the UI uses, as RGB triples. ANSI fallbacks are
/// per-role, not per-palette: in a 16-color terminal all palettes look the
/// same, which is the best a 16-color terminal can do.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub name: &'static str,
    /// True when the palette is designed for a light terminal background.
    pub light: bool,
    pub text: Rgb,
    pub dim: Rgb,
    pub border: Rgb,
    pub border_focused: Rgb,
    pub good: Rgb,
    pub warn: Rgb,
    pub bad: Rgb,
    pub codex: Rgb,
    pub codex_dim: Rgb,
    pub claude: Rgb,
    pub claude_dim: Rgb,
    pub custom: Rgb,
    pub custom_dim: Rgb,
    pub fable: Rgb,
    pub opus: Rgb,
    pub sonnet: Rgb,
    pub haiku: Rgb,
    pub gpt: Rgb,
    pub other_model: Rgb,
}

/// The original lmtop look.
pub const DARK: Palette = Palette {
    name: "dark",
    light: false,
    text: (214, 216, 222),
    dim: (120, 124, 138),
    border: (70, 74, 90),
    border_focused: (140, 150, 190),
    good: (120, 200, 140),
    warn: (240, 190, 100),
    bad: (240, 105, 105),
    codex: (122, 195, 255),
    codex_dim: (56, 108, 151),
    claude: (255, 166, 87),
    claude_dim: (148, 94, 47),
    custom: (126, 222, 190),
    custom_dim: (62, 118, 100),
    fable: (214, 143, 255),
    opus: (255, 166, 87),
    sonnet: (255, 209, 128),
    haiku: (170, 219, 160),
    gpt: (122, 195, 255),
    other_model: (160, 160, 170),
};

pub const LIGHT: Palette = Palette {
    name: "light",
    light: true,
    text: (40, 44, 52),
    dim: (130, 134, 146),
    border: (190, 194, 206),
    border_focused: (90, 105, 170),
    good: (46, 140, 80),
    warn: (176, 121, 12),
    bad: (196, 62, 62),
    codex: (22, 110, 190),
    codex_dim: (120, 165, 205),
    claude: (188, 92, 8),
    claude_dim: (214, 164, 124),
    custom: (16, 128, 96),
    custom_dim: (120, 180, 160),
    fable: (130, 60, 190),
    opus: (188, 92, 8),
    sonnet: (200, 138, 30),
    haiku: (58, 128, 70),
    gpt: (22, 110, 190),
    other_model: (110, 112, 122),
};

/// Catppuccin Mocha.
pub const CATPPUCCIN: Palette = Palette {
    name: "catppuccin",
    light: false,
    text: (205, 214, 244),
    dim: (127, 132, 156),
    border: (69, 71, 90),
    border_focused: (137, 180, 250),
    good: (166, 227, 161),
    warn: (249, 226, 175),
    bad: (243, 139, 168),
    codex: (137, 180, 250),
    codex_dim: (69, 96, 138),
    claude: (250, 179, 135),
    claude_dim: (140, 100, 76),
    custom: (148, 226, 213),
    custom_dim: (74, 118, 110),
    fable: (203, 166, 247),
    opus: (250, 179, 135),
    sonnet: (249, 226, 175),
    haiku: (166, 227, 161),
    gpt: (137, 180, 250),
    other_model: (147, 153, 178),
};

/// Gruvbox dark.
pub const GRUVBOX: Palette = Palette {
    name: "gruvbox",
    light: false,
    text: (235, 219, 178),
    dim: (146, 131, 116),
    border: (80, 73, 69),
    border_focused: (131, 165, 152),
    good: (184, 187, 38),
    warn: (250, 189, 47),
    bad: (251, 73, 52),
    codex: (131, 165, 152),
    codex_dim: (69, 90, 84),
    claude: (254, 128, 25),
    claude_dim: (144, 78, 22),
    custom: (142, 192, 124),
    custom_dim: (74, 102, 66),
    fable: (211, 134, 155),
    opus: (254, 128, 25),
    sonnet: (250, 189, 47),
    haiku: (184, 187, 38),
    gpt: (131, 165, 152),
    other_model: (168, 153, 132),
};

pub const NORD: Palette = Palette {
    name: "nord",
    light: false,
    text: (216, 222, 233),
    dim: (110, 118, 138),
    border: (67, 76, 94),
    border_focused: (136, 192, 208),
    good: (163, 190, 140),
    warn: (235, 203, 139),
    bad: (191, 97, 106),
    codex: (136, 192, 208),
    codex_dim: (70, 100, 110),
    claude: (208, 135, 112),
    claude_dim: (118, 80, 68),
    custom: (143, 188, 187),
    custom_dim: (74, 98, 98),
    fable: (180, 142, 173),
    opus: (208, 135, 112),
    sonnet: (235, 203, 139),
    haiku: (163, 190, 140),
    gpt: (129, 161, 193),
    other_model: (140, 148, 164),
};

pub const PALETTES: [&Palette; 5] = [&DARK, &LIGHT, &CATPPUCCIN, &GRUVBOX, &NORD];

impl Palette {
    /// Look a palette up by name; unknown names get the dark default.
    pub fn by_name(name: &str) -> &'static Palette {
        PALETTES
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
            .copied()
            .unwrap_or(&DARK)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub truecolor: bool,
    pub ascii: bool,
    pub palette: &'static Palette,
}

impl Theme {
    pub fn new(ascii: bool) -> Self {
        Self::named(ascii, "dark")
    }

    pub fn named(ascii: bool, name: &str) -> Self {
        let truecolor = std::env::var("COLORTERM")
            .map(|v| v.contains("truecolor") || v.contains("24bit"))
            .unwrap_or(false);
        Theme {
            truecolor,
            ascii,
            palette: Palette::by_name(name),
        }
    }

    fn color(&self, rgb: Rgb, fallback: Color) -> Color {
        if self.truecolor {
            Color::Rgb(rgb.0, rgb.1, rgb.2)
        } else {
            fallback
        }
    }

    pub fn provider(&self, provider: Provider) -> Color {
        match provider {
            Provider::Codex => self.color(self.palette.codex, Color::Cyan),
            Provider::Claude => self.color(self.palette.claude, Color::Yellow),
            Provider::Custom => self.color(self.palette.custom, Color::Magenta),
        }
    }

    pub fn provider_dim(&self, provider: Provider) -> Color {
        match provider {
            Provider::Codex => self.color(self.palette.codex_dim, Color::Blue),
            Provider::Claude => self.color(self.palette.claude_dim, Color::DarkGray),
            Provider::Custom => self.color(self.palette.custom_dim, Color::DarkGray),
        }
    }

    pub fn model(&self, family: ModelFamily) -> Color {
        match family {
            ModelFamily::ClaudeFable => self.color(self.palette.fable, Color::Magenta),
            ModelFamily::ClaudeOpus => self.color(self.palette.opus, Color::Yellow),
            ModelFamily::ClaudeSonnet => self.color(self.palette.sonnet, Color::LightYellow),
            ModelFamily::ClaudeHaiku => self.color(self.palette.haiku, Color::Green),
            ModelFamily::Gpt => self.color(self.palette.gpt, Color::Cyan),
            ModelFamily::Other => self.color(self.palette.other_model, Color::Gray),
        }
    }

    pub fn text(&self) -> Style {
        let fallback = if self.palette.light {
            Color::Black
        } else {
            Color::White
        };
        Style::default().fg(self.color(self.palette.text, fallback))
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.color(self.palette.dim, Color::DarkGray))
    }

    pub fn title(&self) -> Style {
        self.text().add_modifier(Modifier::BOLD)
    }

    pub fn border(&self) -> Style {
        Style::default().fg(self.color(self.palette.border, Color::DarkGray))
    }

    pub fn border_focused(&self) -> Style {
        Style::default().fg(self.color(self.palette.border_focused, Color::Cyan))
    }

    pub fn good(&self) -> Color {
        self.color(self.palette.good, Color::Green)
    }

    pub fn warn(&self) -> Color {
        self.color(self.palette.warn, Color::Yellow)
    }

    pub fn bad(&self) -> Color {
        self.color(self.palette.bad, Color::Red)
    }

    /// Gauge color for a used percentage. In truecolor this is a continuous
    /// blend (green → yellow across 40–75%, yellow → red across 75–95%);
    /// 16-color terminals keep the classic three steps.
    pub fn gauge_color(&self, used_percent: f64) -> Color {
        if !self.truecolor {
            return if used_percent >= 90.0 {
                Color::Red
            } else if used_percent >= 70.0 {
                Color::Yellow
            } else {
                Color::Green
            };
        }
        let p = used_percent.clamp(0.0, 100.0);
        let (from, to, t) = if p < 40.0 {
            (self.palette.good, self.palette.good, 0.0)
        } else if p < 75.0 {
            (self.palette.good, self.palette.warn, (p - 40.0) / 35.0)
        } else if p < 95.0 {
            (self.palette.warn, self.palette.bad, (p - 75.0) / 20.0)
        } else {
            (self.palette.bad, self.palette.bad, 1.0)
        };
        Color::Rgb(
            lerp(from.0, to.0, t),
            lerp(from.1, to.1, t),
            lerp(from.2, to.2, t),
        )
    }

    pub fn status(&self, status: CollectorStatus) -> (Color, &'static str) {
        match status {
            CollectorStatus::Ok => (self.good(), "ok"),
            CollectorStatus::Degraded => (self.warn(), "degraded"),
            CollectorStatus::Error => (self.bad(), "error"),
            CollectorStatus::Disabled => (self.dim().fg.unwrap_or(Color::DarkGray), "disabled"),
            CollectorStatus::Unavailable => {
                (self.dim().fg.unwrap_or(Color::DarkGray), "unavailable")
            }
        }
    }

    pub fn freshness(&self, freshness: Freshness) -> Color {
        match freshness {
            Freshness::Fresh => self.good(),
            Freshness::Stale => self.warn(),
            Freshness::Unavailable => self.dim().fg.unwrap_or(Color::DarkGray),
        }
    }
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t.clamp(0.0, 1.0)).round() as u8
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

/// Duration until a reset, like "2h05m" or "3d 4h". Under ten minutes the
/// seconds are shown ("4m32s") so countdowns visibly tick.
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
    } else if minutes >= 10 {
        format!("{minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m{:02}s", seconds % 60)
    } else {
        format!("{}s", seconds % 60)
    }
}

/// Unicode sparkline of a value series, scaled to its own min/max. Returns
/// `None` in ASCII mode or when there are fewer than two points.
pub fn sparkline(values: &[f64], width: usize, ascii: bool) -> Option<String> {
    if ascii || values.len() < 2 || width == 0 {
        return None;
    }
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    // Resample to `width` buckets, keeping the newest values right-aligned.
    let take = values.len().min(width * 8); // cap work, plenty of resolution
    let values = &values[values.len() - take..];
    let n = values.len().min(width);
    let chunk = values.len() as f64 / n as f64;
    let mut points = Vec::with_capacity(n);
    for i in 0..n {
        let start = (i as f64 * chunk) as usize;
        let end = (((i + 1) as f64 * chunk) as usize).max(start + 1);
        let slice = &values[start..end.min(values.len())];
        points.push(slice.iter().copied().sum::<f64>() / slice.len() as f64);
    }
    let min = points.iter().copied().fold(f64::INFINITY, f64::min);
    let max = points.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(f64::EPSILON);
    Some(
        points
            .iter()
            .map(|v| LEVELS[(((v - min) / span) * 7.0).round() as usize])
            .collect(),
    )
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
        // Seconds appear under ten minutes so countdowns tick.
        assert_eq!(fmt_duration_until(272), "4m32s");
        assert_eq!(fmt_duration_until(59), "59s");
        assert_eq!(fmt_duration_until(660), "11m");
    }

    #[test]
    fn palette_lookup() {
        assert_eq!(Palette::by_name("nord").name, "nord");
        assert_eq!(Palette::by_name("Catppuccin").name, "catppuccin");
        assert_eq!(Palette::by_name("bogus").name, "dark");
        assert_eq!(Palette::by_name("").name, "dark");
    }

    #[test]
    fn gauge_color_is_stepped_without_truecolor() {
        let t = Theme {
            truecolor: false,
            ascii: false,
            palette: &DARK,
        };
        assert_eq!(t.gauge_color(10.0), Color::Green);
        assert_eq!(t.gauge_color(75.0), Color::Yellow);
        assert_eq!(t.gauge_color(95.0), Color::Red);
    }

    #[test]
    fn gauge_color_blends_with_truecolor() {
        let t = Theme {
            truecolor: true,
            ascii: false,
            palette: &DARK,
        };
        assert_eq!(
            t.gauge_color(0.0),
            Color::Rgb(DARK.good.0, DARK.good.1, DARK.good.2)
        );
        assert_eq!(
            t.gauge_color(100.0),
            Color::Rgb(DARK.bad.0, DARK.bad.1, DARK.bad.2)
        );
        // Midway between good and warn is neither anchor.
        let mid = t.gauge_color(57.5);
        assert_ne!(mid, t.gauge_color(0.0));
        assert_ne!(mid, t.gauge_color(74.9));
    }

    #[test]
    fn sparkline_scales_and_falls_back() {
        assert!(sparkline(&[1.0, 2.0], 8, true).is_none());
        assert!(sparkline(&[1.0], 8, false).is_none());
        let s = sparkline(&[0.0, 50.0, 100.0], 3, false).unwrap();
        assert_eq!(s.chars().count(), 3);
        assert!(s.starts_with('▁'));
        assert!(s.ends_with('█'));
    }
}

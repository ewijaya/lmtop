//! Named color palettes plus formatting helpers. `ui.theme` in the config
//! picks a palette by name: `dark` and `light` are compiled in as Rust,
//! every file under the repo's `themes/` directory is compiled in as TOML
//! (mostly mechanical conversions of btop's theme set), and users can add
//! or override themes with `<config dir>/themes/<name>.toml`. Unknown
//! names fall back to dark; the `t`/`T` keys cycle the loaded set at
//! runtime. Colors degrade with the detected [`ColorDepth`]: exact in
//! truecolor, quantized to the xterm cube in 256-color terminals, and
//! per-role ANSI fallbacks (identical across themes) at 16 colors.

use crate::domain::{CollectorStatus, Freshness, ModelFamily, Provider};
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// A truecolor triple, deserialized from `"#rrggbb"` (or `"#rgb"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_hex(&text).ok_or_else(|| {
            serde::de::Error::custom(format!("invalid color {text:?}, expected \"#rrggbb\""))
        })
    }
}

fn parse_hex(text: &str) -> Option<Rgb> {
    let hex = text.trim().strip_prefix('#')?;
    let channel = |i: usize| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok();
    let nibble = |i: usize| {
        u8::from_str_radix(&hex[i..i + 1], 16)
            .ok()
            .map(|v| v * 16 + v)
    };
    match hex.len() {
        6 => Some(Rgb(channel(0)?, channel(1)?, channel(2)?)),
        3 => Some(Rgb(nibble(0)?, nibble(1)?, nibble(2)?)),
        _ => None,
    }
}

/// Every color role the UI uses. ANSI fallbacks are per-role, not
/// per-palette: in a 16-color terminal all palettes look the same, which
/// is the best a 16-color terminal can do. A theme file may omit roles;
/// missing ones keep the dark palette's colors.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Palette {
    /// From the builtin registry or the theme file's stem, never the file
    /// body.
    #[serde(skip)]
    pub name: String,
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

impl Default for Palette {
    fn default() -> Self {
        dark()
    }
}

/// The original lmtop look, and the source of defaults for partial theme
/// files.
fn dark() -> Palette {
    Palette {
        name: "dark".into(),
        light: false,
        text: Rgb(214, 216, 222),
        dim: Rgb(120, 124, 138),
        border: Rgb(70, 74, 90),
        border_focused: Rgb(140, 150, 190),
        good: Rgb(120, 200, 140),
        warn: Rgb(240, 190, 100),
        bad: Rgb(240, 105, 105),
        codex: Rgb(122, 195, 255),
        codex_dim: Rgb(56, 108, 151),
        claude: Rgb(255, 166, 87),
        claude_dim: Rgb(148, 94, 47),
        custom: Rgb(126, 222, 190),
        custom_dim: Rgb(62, 118, 100),
        fable: Rgb(214, 143, 255),
        opus: Rgb(255, 166, 87),
        sonnet: Rgb(255, 209, 128),
        haiku: Rgb(170, 219, 160),
        gpt: Rgb(122, 195, 255),
        other_model: Rgb(160, 160, 170),
    }
}

fn light() -> Palette {
    Palette {
        name: "light".into(),
        light: true,
        text: Rgb(40, 44, 52),
        dim: Rgb(130, 134, 146),
        border: Rgb(190, 194, 206),
        border_focused: Rgb(90, 105, 170),
        good: Rgb(46, 140, 80),
        warn: Rgb(176, 121, 12),
        bad: Rgb(196, 62, 62),
        codex: Rgb(22, 110, 190),
        codex_dim: Rgb(120, 165, 205),
        claude: Rgb(188, 92, 8),
        claude_dim: Rgb(214, 164, 124),
        custom: Rgb(16, 128, 96),
        custom_dim: Rgb(120, 180, 160),
        fable: Rgb(130, 60, 190),
        opus: Rgb(188, 92, 8),
        sonnet: Rgb(200, 138, 30),
        haiku: Rgb(58, 128, 70),
        gpt: Rgb(22, 110, 190),
        other_model: Rgb(110, 112, 122),
    }
}

/// Theme files compiled into the binary. Regenerate the btop conversions
/// with `scripts/btop2lmtop.py`.
const EMBEDDED: &[(&str, &str)] = &[
    ("adapta", include_str!("../../themes/adapta.toml")),
    ("adwaita", include_str!("../../themes/adwaita.toml")),
    (
        "adwaita-dark",
        include_str!("../../themes/adwaita-dark.toml"),
    ),
    ("ayu", include_str!("../../themes/ayu.toml")),
    ("catppuccin", include_str!("../../themes/catppuccin.toml")),
    ("dracula", include_str!("../../themes/dracula.toml")),
    ("dusklight", include_str!("../../themes/dusklight.toml")),
    (
        "elementarish",
        include_str!("../../themes/elementarish.toml"),
    ),
    (
        "everforest-dark-hard",
        include_str!("../../themes/everforest-dark-hard.toml"),
    ),
    (
        "everforest-dark-medium",
        include_str!("../../themes/everforest-dark-medium.toml"),
    ),
    (
        "everforest-light-medium",
        include_str!("../../themes/everforest-light-medium.toml"),
    ),
    ("flat-remix", include_str!("../../themes/flat-remix.toml")),
    (
        "flat-remix-light",
        include_str!("../../themes/flat-remix-light.toml"),
    ),
    (
        "flexoki-dark",
        include_str!("../../themes/flexoki-dark.toml"),
    ),
    (
        "flexoki-light",
        include_str!("../../themes/flexoki-light.toml"),
    ),
    ("gotham", include_str!("../../themes/gotham.toml")),
    ("greyscale", include_str!("../../themes/greyscale.toml")),
    ("gruvbox", include_str!("../../themes/gruvbox.toml")),
    (
        "gruvbox_dark",
        include_str!("../../themes/gruvbox_dark.toml"),
    ),
    (
        "gruvbox_dark_v2",
        include_str!("../../themes/gruvbox_dark_v2.toml"),
    ),
    (
        "gruvbox_light",
        include_str!("../../themes/gruvbox_light.toml"),
    ),
    (
        "gruvbox_material_dark",
        include_str!("../../themes/gruvbox_material_dark.toml"),
    ),
    ("horizon", include_str!("../../themes/horizon.toml")),
    (
        "HotPurpleTrafficLight",
        include_str!("../../themes/HotPurpleTrafficLight.toml"),
    ),
    (
        "kanagawa-dragon",
        include_str!("../../themes/kanagawa-dragon.toml"),
    ),
    (
        "kanagawa-lotus",
        include_str!("../../themes/kanagawa-lotus.toml"),
    ),
    (
        "kanagawa-wave",
        include_str!("../../themes/kanagawa-wave.toml"),
    ),
    ("kyli0x", include_str!("../../themes/kyli0x.toml")),
    (
        "matcha-dark-sea",
        include_str!("../../themes/matcha-dark-sea.toml"),
    ),
    ("monokai", include_str!("../../themes/monokai.toml")),
    ("night-owl", include_str!("../../themes/night-owl.toml")),
    ("nord", include_str!("../../themes/nord.toml")),
    ("onedark", include_str!("../../themes/onedark.toml")),
    ("orange", include_str!("../../themes/orange.toml")),
    ("paper", include_str!("../../themes/paper.toml")),
    (
        "phoenix-night",
        include_str!("../../themes/phoenix-night.toml"),
    ),
    (
        "solarized_dark",
        include_str!("../../themes/solarized_dark.toml"),
    ),
    (
        "solarized_light",
        include_str!("../../themes/solarized_light.toml"),
    ),
    ("tokyo-night", include_str!("../../themes/tokyo-night.toml")),
    ("tokyo-storm", include_str!("../../themes/tokyo-storm.toml")),
    (
        "tomorrow-night",
        include_str!("../../themes/tomorrow-night.toml"),
    ),
    ("twilight", include_str!("../../themes/twilight.toml")),
    ("whiteout", include_str!("../../themes/whiteout.toml")),
];

/// All compiled-in palettes: dark, light, then the embedded set sorted by
/// name. Parsed once; a bad embedded file is a build defect (every one is
/// parsed by tests), so this panics rather than papering over it.
pub fn builtin_palettes() -> &'static [Palette] {
    static CACHE: OnceLock<Vec<Palette>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut palettes = vec![dark(), light()];
        for (name, text) in EMBEDDED {
            let mut palette: Palette = toml::from_str(text)
                .unwrap_or_else(|err| panic!("embedded theme {name} is invalid: {err}"));
            palette.name = (*name).into();
            palettes.push(palette);
        }
        sort_palettes(&mut palettes);
        palettes
    })
}

/// dark and light lead (they are the fallback pair); everything else is
/// alphabetical. This is the `t`/`T` cycling order.
fn sort_palettes(palettes: &mut [Palette]) {
    palettes.sort_by_key(|p| {
        let rank = match p.name.as_str() {
            "dark" => 0,
            "light" => 1,
            _ => 2,
        };
        (rank, p.name.to_ascii_lowercase())
    });
}

/// The user theme directory, `<config dir>/themes`.
pub fn themes_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", crate::branding::APP_DIR)
        .map(|dirs| dirs.config_dir().join("themes"))
}

/// Builtins plus `*.toml` files from `dir`; a user theme named like a
/// builtin replaces it. Unreadable or invalid files are logged and
/// skipped, never fatal — a broken theme must not take the monitor down.
fn load_palettes(dir: Option<&Path>) -> Vec<Palette> {
    let mut palettes = builtin_palettes().to_vec();
    let Some(read) = dir.and_then(|d| std::fs::read_dir(d).ok()) else {
        return palettes;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| toml::from_str::<Palette>(&text).map_err(|e| e.to_string()));
        match parsed {
            Ok(mut palette) => {
                palette.name = name.into();
                match palettes
                    .iter_mut()
                    .find(|p| p.name.eq_ignore_ascii_case(name))
                {
                    Some(slot) => *slot = palette,
                    None => palettes.push(palette),
                }
            }
            Err(err) => tracing::warn!("skipping theme {}: {err}", path.display()),
        }
    }
    sort_palettes(&mut palettes);
    palettes
}

/// How many colors the terminal can show. Truecolor renders palettes
/// exactly; 256-color renders them quantized to the xterm cube (close
/// enough to tell themes apart); 16-color drops to per-role ANSI
/// fallbacks, where every theme looks the same. The header shows the
/// active depth whenever it is degraded, rather than failing silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Indexed256,
    Ansi16,
}

impl ColorDepth {
    /// Detect from `COLORTERM`/`TERM`. SSH does not forward `COLORTERM`,
    /// so a truecolor terminal often shows up here as its `TERM` string
    /// only — `ui.color_depth` exists to override the guess.
    fn detect(colorterm: &str, term: &str) -> Self {
        let colorterm = colorterm.to_ascii_lowercase();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return ColorDepth::TrueColor;
        }
        let term = term.to_ascii_lowercase();
        if term.contains("truecolor")
            || term.contains("direct")
            || term.contains("kitty")
            || term.contains("ghostty")
            || term.contains("wezterm")
            || term.contains("alacritty")
            || term.contains("iterm")
        {
            return ColorDepth::TrueColor;
        }
        if term.contains("256") {
            return ColorDepth::Indexed256;
        }
        ColorDepth::Ansi16
    }

    fn from_env() -> Self {
        Self::detect(
            &std::env::var("COLORTERM").unwrap_or_default(),
            &std::env::var("TERM").unwrap_or_default(),
        )
    }

    /// Config override: "truecolor", "256", or "16"; anything else
    /// (including the default "auto") means keep the detected depth.
    pub fn from_config(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" => Some(ColorDepth::TrueColor),
            "256" | "256color" => Some(ColorDepth::Indexed256),
            "16" | "ansi" => Some(ColorDepth::Ansi16),
            _ => None,
        }
    }

    /// Suffix for the header's theme-name readout; empty at full depth.
    pub fn label(self) -> &'static str {
        match self {
            ColorDepth::TrueColor => "",
            ColorDepth::Indexed256 => "·256",
            ColorDepth::Ansi16 => "·16",
        }
    }
}

/// Nearest xterm-256 index for an RGB color: the 6×6×6 color cube
/// (16–231) or the grayscale ramp (232–255), whichever is closer.
fn xterm256(rgb: Rgb) -> u8 {
    // Cube channel levels are 0, 95, 135, 175, 215, 255.
    fn cube_component(v: u8) -> (u8, u8) {
        let idx = if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v as u16 - 35) as u8 / 40
        };
        (idx, if idx == 0 { 0 } else { 55 + idx * 40 })
    }
    let (ri, rv) = cube_component(rgb.0);
    let (gi, gv) = cube_component(rgb.1);
    let (bi, bv) = cube_component(rgb.2);
    let cube_index = 16 + 36 * ri + 6 * gi + bi;
    let dist = |a: Rgb, b: Rgb| -> u32 {
        let d = |x: u8, y: u8| (x as i32 - y as i32).pow(2) as u32;
        d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
    };
    let cube_dist = dist(rgb, Rgb(rv, gv, bv));
    // Grayscale ramp: 232 + i has value 8 + 10i, i in 0..24.
    let gray_avg = ((rgb.0 as u16 + rgb.1 as u16 + rgb.2 as u16) / 3) as u8;
    let gray_i = (gray_avg.saturating_sub(3) / 10).min(23);
    let gray_v = 8 + 10 * gray_i;
    if dist(rgb, Rgb(gray_v, gray_v, gray_v)) < cube_dist {
        232 + gray_i
    } else {
        cube_index
    }
}

/// Chart drawing symbol for the rate/quota chart, `ui.graph_symbol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphSymbol {
    /// Braille dots, 2×4 sub-cells — highest resolution.
    #[default]
    Braille,
    /// Half-block elements, 2 vertical sub-cells — medium resolution,
    /// broader font compatibility.
    Block,
    /// Plain dots — lowest resolution, maximum terminal compatibility.
    Tty,
}

impl GraphSymbol {
    /// Unknown names fall back to braille, like unknown theme names fall
    /// back to dark.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "block" => GraphSymbol::Block,
            "tty" => GraphSymbol::Tty,
            _ => GraphSymbol::Braille,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub depth: ColorDepth,
    pub ascii: bool,
    pub graph_symbol: GraphSymbol,
    palettes: Arc<Vec<Palette>>,
    index: usize,
}

impl Theme {
    /// Builtin palettes only, starting on dark. Used by tests; the real
    /// entry point is [`Theme::named`].
    pub fn new(ascii: bool) -> Self {
        Self::with_palettes(ascii, "dark", builtin_palettes().to_vec())
    }

    /// The full palette set — builtins plus the user's theme files —
    /// starting on `name` (unknown names fall back to dark).
    pub fn named(ascii: bool, name: &str) -> Self {
        Self::with_palettes(ascii, name, load_palettes(themes_dir().as_deref()))
    }

    fn with_palettes(ascii: bool, name: &str, palettes: Vec<Palette>) -> Self {
        let index = palettes
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name.trim()))
            .unwrap_or(0);
        Theme {
            depth: ColorDepth::from_env(),
            ascii,
            graph_symbol: GraphSymbol::default(),
            palettes: Arc::new(palettes),
            index,
        }
    }

    pub fn palette(&self) -> &Palette {
        &self.palettes[self.index]
    }

    /// The chart marker: `ui.graph_symbol`, except ASCII mode always gets
    /// the lowest-tech marker.
    pub fn marker(&self) -> ratatui::symbols::Marker {
        use ratatui::symbols::Marker;
        if self.ascii {
            return Marker::Dot;
        }
        match self.graph_symbol {
            GraphSymbol::Braille => Marker::Braille,
            GraphSymbol::Block => Marker::HalfBlock,
            GraphSymbol::Tty => Marker::Dot,
        }
    }

    /// Step to the next (+1) or previous (-1) palette — the `t`/`T` keys.
    /// The switch lasts for the session; `ui.theme` makes it permanent.
    pub fn cycle(&mut self, step: i32) {
        let count = self.palettes.len() as i32;
        self.index = (self.index as i32 + step).rem_euclid(count) as usize;
    }

    fn color(&self, rgb: Rgb, fallback: Color) -> Color {
        match self.depth {
            ColorDepth::TrueColor => Color::Rgb(rgb.0, rgb.1, rgb.2),
            ColorDepth::Indexed256 => Color::Indexed(xterm256(rgb)),
            ColorDepth::Ansi16 => fallback,
        }
    }

    pub fn provider(&self, provider: Provider) -> Color {
        match provider {
            Provider::Codex => self.color(self.palette().codex, Color::Cyan),
            Provider::Claude => self.color(self.palette().claude, Color::Yellow),
            Provider::Custom => self.color(self.palette().custom, Color::Magenta),
        }
    }

    pub fn provider_dim(&self, provider: Provider) -> Color {
        match provider {
            Provider::Codex => self.color(self.palette().codex_dim, Color::Blue),
            Provider::Claude => self.color(self.palette().claude_dim, Color::DarkGray),
            Provider::Custom => self.color(self.palette().custom_dim, Color::DarkGray),
        }
    }

    pub fn model(&self, family: ModelFamily) -> Color {
        match family {
            ModelFamily::ClaudeFable => self.color(self.palette().fable, Color::Magenta),
            ModelFamily::ClaudeOpus => self.color(self.palette().opus, Color::Yellow),
            ModelFamily::ClaudeSonnet => self.color(self.palette().sonnet, Color::LightYellow),
            ModelFamily::ClaudeHaiku => self.color(self.palette().haiku, Color::Green),
            ModelFamily::Gpt => self.color(self.palette().gpt, Color::Cyan),
            ModelFamily::Other => self.color(self.palette().other_model, Color::Gray),
        }
    }

    pub fn text(&self) -> Style {
        let fallback = if self.palette().light {
            Color::Black
        } else {
            Color::White
        };
        Style::default().fg(self.color(self.palette().text, fallback))
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.color(self.palette().dim, Color::DarkGray))
    }

    pub fn title(&self) -> Style {
        self.text().add_modifier(Modifier::BOLD)
    }

    pub fn border(&self) -> Style {
        Style::default().fg(self.color(self.palette().border, Color::DarkGray))
    }

    pub fn border_focused(&self) -> Style {
        Style::default().fg(self.color(self.palette().border_focused, Color::Cyan))
    }

    pub fn good(&self) -> Color {
        self.color(self.palette().good, Color::Green)
    }

    pub fn warn(&self) -> Color {
        self.color(self.palette().warn, Color::Yellow)
    }

    pub fn bad(&self) -> Color {
        self.color(self.palette().bad, Color::Red)
    }

    /// Gauge color for a used percentage: a continuous blend (green →
    /// yellow across 40–75%, yellow → red across 75–95%), quantized in
    /// 256-color mode; 16-color terminals keep the classic three steps.
    pub fn gauge_color(&self, used_percent: f64) -> Color {
        let stepped = if used_percent >= 90.0 {
            Color::Red
        } else if used_percent >= 70.0 {
            Color::Yellow
        } else {
            Color::Green
        };
        let p = used_percent.clamp(0.0, 100.0);
        let palette = self.palette();
        let (from, to, t) = if p < 40.0 {
            (palette.good, palette.good, 0.0)
        } else if p < 75.0 {
            (palette.good, palette.warn, (p - 40.0) / 35.0)
        } else if p < 95.0 {
            (palette.warn, palette.bad, (p - 75.0) / 20.0)
        } else {
            (palette.bad, palette.bad, 1.0)
        };
        let blended = Rgb(
            lerp(from.0, to.0, t),
            lerp(from.1, to.1, t),
            lerp(from.2, to.2, t),
        );
        self.color(blended, stepped)
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

    fn test_theme(depth: ColorDepth, palette: Palette) -> Theme {
        Theme {
            depth,
            ascii: false,
            graph_symbol: GraphSymbol::default(),
            palettes: Arc::new(vec![palette]),
            index: 0,
        }
    }

    #[test]
    fn detects_color_depth() {
        use ColorDepth::*;
        assert_eq!(ColorDepth::detect("truecolor", "xterm-256color"), TrueColor);
        assert_eq!(ColorDepth::detect("24bit", "screen"), TrueColor);
        // SSH strips COLORTERM; TERM alone must still find the right tier.
        assert_eq!(ColorDepth::detect("", "xterm-256color"), Indexed256);
        assert_eq!(ColorDepth::detect("", "tmux-256color"), Indexed256);
        assert_eq!(ColorDepth::detect("", "xterm-kitty"), TrueColor);
        assert_eq!(ColorDepth::detect("", "xterm-direct"), TrueColor);
        assert_eq!(ColorDepth::detect("", "screen"), Ansi16);
        assert_eq!(ColorDepth::detect("", ""), Ansi16);
        // Config override parsing; "auto"/unknown mean no override.
        assert_eq!(ColorDepth::from_config("truecolor"), Some(TrueColor));
        assert_eq!(ColorDepth::from_config("256"), Some(Indexed256));
        assert_eq!(ColorDepth::from_config("16"), Some(Ansi16));
        assert_eq!(ColorDepth::from_config("auto"), None);
        assert_eq!(ColorDepth::from_config("bogus"), None);
    }

    #[test]
    fn quantizes_to_xterm256() {
        assert_eq!(xterm256(Rgb(0, 0, 0)), 16); // cube black
        assert_eq!(xterm256(Rgb(255, 255, 255)), 231); // cube white
        assert_eq!(xterm256(Rgb(255, 0, 0)), 196); // pure red
        assert_eq!(xterm256(Rgb(0, 255, 0)), 46); // pure green
        assert_eq!(xterm256(Rgb(0, 0, 255)), 21); // pure blue
        assert_eq!(xterm256(Rgb(128, 128, 128)), 244); // mid gray → ramp
        assert_eq!(xterm256(Rgb(95, 135, 175)), 67); // exact cube color
    }

    #[test]
    fn color_depth_changes_rendering() {
        let rgb = Rgb(122, 195, 255);
        let truecolor = test_theme(ColorDepth::TrueColor, dark());
        assert_eq!(truecolor.color(rgb, Color::Cyan), Color::Rgb(122, 195, 255));
        let indexed = test_theme(ColorDepth::Indexed256, dark());
        assert_eq!(indexed.color(rgb, Color::Cyan), Color::Indexed(117));
        let ansi = test_theme(ColorDepth::Ansi16, dark());
        assert_eq!(ansi.color(rgb, Color::Cyan), Color::Cyan);
        // The gauge blend quantizes too instead of dropping to steps.
        assert!(matches!(
            indexed.gauge_color(57.5),
            Color::Indexed(i) if i >= 16
        ));
    }

    #[test]
    fn graph_symbol_maps_to_markers() {
        use ratatui::symbols::Marker;
        assert_eq!(GraphSymbol::from_name("braille"), GraphSymbol::Braille);
        assert_eq!(GraphSymbol::from_name(" Block "), GraphSymbol::Block);
        assert_eq!(GraphSymbol::from_name("tty"), GraphSymbol::Tty);
        assert_eq!(GraphSymbol::from_name("bogus"), GraphSymbol::Braille);
        let mut theme = Theme::new(false);
        assert_eq!(theme.marker(), Marker::Braille);
        theme.graph_symbol = GraphSymbol::Block;
        assert_eq!(theme.marker(), Marker::HalfBlock);
        theme.graph_symbol = GraphSymbol::Tty;
        assert_eq!(theme.marker(), Marker::Dot);
        // ASCII mode wins over any configured symbol.
        let mut ascii = Theme::new(true);
        ascii.graph_symbol = GraphSymbol::Braille;
        assert_eq!(ascii.marker(), Marker::Dot);
    }

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
    fn parses_hex_colors() {
        assert_eq!(parse_hex("#ff8000"), Some(Rgb(255, 128, 0)));
        assert_eq!(parse_hex(" #FA5 "), Some(Rgb(255, 170, 85)));
        assert_eq!(parse_hex("ff8000"), None);
        assert_eq!(parse_hex("#ff80"), None);
        assert_eq!(parse_hex("#gg8000"), None);
    }

    /// Every embedded theme must parse — builtin_palettes panics otherwise,
    /// so calling it is the assertion.
    #[test]
    fn embedded_themes_parse_and_have_unique_names() {
        let palettes = builtin_palettes();
        assert_eq!(palettes.len(), 2 + EMBEDDED.len());
        assert_eq!(palettes[0].name, "dark");
        assert_eq!(palettes[1].name, "light");
        let mut names: Vec<String> = palettes
            .iter()
            .map(|p| p.name.to_ascii_lowercase())
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), palettes.len(), "duplicate theme names");
        // Spot checks: light themes converted from btop declare themselves.
        let solarized = palettes
            .iter()
            .find(|p| p.name == "solarized_light")
            .unwrap();
        assert!(solarized.light);
        assert!(!palettes.iter().find(|p| p.name == "dracula").unwrap().light);
    }

    #[test]
    fn palette_lookup_falls_back_to_dark() {
        let theme = Theme::new(false);
        assert_eq!(theme.palette().name, "dark");
        let nord = Theme::with_palettes(false, "Nord", builtin_palettes().to_vec());
        assert_eq!(nord.palette().name, "nord");
        let bogus = Theme::with_palettes(false, "bogus", builtin_palettes().to_vec());
        assert_eq!(bogus.palette().name, "dark");
    }

    #[test]
    fn cycling_wraps_both_directions() {
        let mut theme = Theme::new(false);
        let count = builtin_palettes().len();
        assert_eq!(theme.palette().name, "dark");
        theme.cycle(-1);
        assert_ne!(theme.palette().name, "dark", "backward wrap");
        theme.cycle(1);
        assert_eq!(theme.palette().name, "dark");
        for _ in 0..count {
            theme.cycle(1);
        }
        assert_eq!(theme.palette().name, "dark", "full forward cycle");
    }

    #[test]
    fn user_themes_load_and_override() {
        let dir = std::env::temp_dir().join(format!("lmtop-theme-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Partial theme: unset roles fall back to the dark palette.
        std::fs::write(dir.join("mytheme.toml"), "good = \"#123456\"\n").unwrap();
        // Overrides the builtin of the same name.
        std::fs::write(dir.join("dracula.toml"), "bad = \"#ff0000\"\n").unwrap();
        // Invalid files are skipped, not fatal.
        std::fs::write(dir.join("broken.toml"), "bad = \"notacolor\"\n").unwrap();
        let palettes = load_palettes(Some(&dir));
        std::fs::remove_dir_all(&dir).unwrap();

        let mine = palettes.iter().find(|p| p.name == "mytheme").unwrap();
        assert_eq!(mine.good, Rgb(0x12, 0x34, 0x56));
        assert_eq!(mine.text, dark().text);
        let dracula = palettes.iter().find(|p| p.name == "dracula").unwrap();
        assert_eq!(dracula.bad, Rgb(255, 0, 0));
        assert!(!palettes.iter().any(|p| p.name == "broken"));
        assert_eq!(
            palettes.len(),
            builtin_palettes().len() + 1,
            "override replaced, mytheme appended, broken skipped"
        );
    }

    #[test]
    fn gauge_color_is_stepped_without_truecolor() {
        let t = test_theme(ColorDepth::Ansi16, dark());
        assert_eq!(t.gauge_color(10.0), Color::Green);
        assert_eq!(t.gauge_color(75.0), Color::Yellow);
        assert_eq!(t.gauge_color(95.0), Color::Red);
    }

    #[test]
    fn gauge_color_blends_with_truecolor() {
        let t = test_theme(ColorDepth::TrueColor, dark());
        let d = dark();
        assert_eq!(t.gauge_color(0.0), Color::Rgb(d.good.0, d.good.1, d.good.2));
        assert_eq!(t.gauge_color(100.0), Color::Rgb(d.bad.0, d.bad.1, d.bad.2));
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

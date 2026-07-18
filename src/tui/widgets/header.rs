use crate::alerts::Severity;
use crate::app::App;
use crate::tui::theme::{Theme, fmt_age};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, now: DateTime<Utc>) {
    let mut spans = vec![
        Span::styled(format!(" {} ", crate::branding::APP_NAME), theme.title()),
        Span::styled(format!("v{} ", env!("CARGO_PKG_VERSION")), theme.dim()),
    ];
    if app.paused {
        spans.push(Span::styled(
            " PAUSED ",
            Style::default()
                .fg(ratatui::style::Color::Black)
                .bg(theme.warn())
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(
            format!(" refresh {}s ", app.refresh_secs),
            theme.dim(),
        ));
    }
    // A fresh alert takes the header over; provider health otherwise.
    if let Some(alert) = app.flash_alert(now) {
        let color = match alert.severity {
            Severity::Critical => theme.bad(),
            Severity::Warning => theme.warn(),
        };
        spans.push(Span::styled(
            format!(" ⚠ {} — {} ", alert.title, alert.body),
            Style::default()
                .fg(ratatui::style::Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        for provider in &app.enabled_providers {
            let (color, label) = match app.provider(*provider) {
                Some(p) => theme.status(p.health.status),
                None => (theme.dim().fg.unwrap_or_default(), "starting"),
            };
            spans.push(Span::styled(
                format!(" {} ", provider.display_name()),
                Style::default().fg(theme.provider(*provider)),
            ));
            spans.push(Span::styled(
                format!("{label} "),
                Style::default().fg(color),
            ));
        }
    }
    let clock = now
        .with_timezone(&chrono::Local)
        .format("%H:%M:%S")
        .to_string();
    let left = Line::from(spans);
    // Theme name beside the clock (feedback for the t/T cycle keys), with
    // the color depth appended when it is degraded — at ·16 every theme
    // looks identical, and that should be visible, not mysterious. Only
    // when the row is wide enough not to collide with the left half.
    let right_text = if area.width >= 90 {
        format!("{}{}  {clock} ", theme.palette().name, theme.depth.label())
    } else {
        format!("{clock} ")
    };
    let right = Line::from(Span::styled(right_text, theme.dim())).right_aligned();
    frame.render_widget(Paragraph::new(left), area);
    frame.render_widget(Paragraph::new(right), area);
}

pub fn render_footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, now: DateTime<Utc>) {
    let keys: &[(&str, &str)] = if area.width < 80 {
        &[("1-4", "view"), ("?", "help"), ("q", "quit")]
    } else if area.width < 110 {
        &[
            ("1", "codex"),
            ("2", "claude"),
            ("3", "all"),
            ("4", "plan"),
            ("t", "theme"),
            ("?", "help"),
            ("Enter", "detail"),
            ("/", "filter"),
            ("q", "quit"),
        ]
    } else {
        &[
            ("1", "codex"),
            ("2", "claude"),
            ("3", "all"),
            ("4", "planner"),
            ("t", "theme"),
            ("?", "help"),
            ("Enter", "detail"),
            ("o", "sort"),
            ("/", "filter"),
            ("r", "refresh"),
            ("p", "pause"),
            ("q", "quit"),
        ]
    };
    // Right side first: freshness per provider is the higher-value half,
    // so it claims its width and the key hints take what is left.
    let mut right = Vec::new();
    for provider in &app.enabled_providers {
        let freshness = app.freshness(*provider, now);
        let age = app
            .provider(*provider)
            .and_then(|p| p.health.last_scan)
            .map(|t| fmt_age(now.signed_duration_since(t).num_seconds()))
            .unwrap_or_else(|| "-".into());
        right.push(Span::styled(
            format!("{} ", provider.display_name()),
            Style::default().fg(theme.provider_dim(*provider)),
        ));
        right.push(Span::styled(
            format!("{} {age} ", freshness.label()),
            Style::default().fg(theme.freshness(freshness)),
        ));
    }
    let right_line = Line::from(right);
    let right_width = right_line.width() as u16;

    // Drop key hints from the right end until they fit beside the
    // freshness block: two paragraphs share this row, so the left half
    // must be told how much room it actually has. One extra cell is
    // reserved so the last hint never sits flush against the freshness
    // text (its own trailing space alone reads as a collision).
    let key_budget = area.width.saturating_sub(right_width + 1);
    let mut spans = Vec::new();
    let mut used = 0u16;
    for (key, label) in keys {
        let cost = (key.chars().count() + label.chars().count() + 3) as u16;
        if used + cost > key_budget {
            break;
        }
        used += cost;
        spans.push(Span::styled(
            format!(" {key} "),
            theme.title().add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::styled(format!("{label} "), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    frame.render_widget(Paragraph::new(right_line.right_aligned()), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::domain::{CollectorHealth, CollectorStatus, Provider, ProviderSnapshot};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    fn app_with_providers(now: DateTime<Utc>) -> App {
        let mut app = App::new(now, 5);
        for provider in [Provider::Codex, Provider::Claude] {
            let snap = ProviderSnapshot::empty(
                provider,
                CollectorHealth {
                    status: CollectorStatus::Ok,
                    message: None,
                    last_scan: Some(now),
                    files_scanned: 1,
                    parse_errors: 0,
                },
            );
            app.apply_update(snap, now);
        }
        app
    }

    /// The footer draws two paragraphs into one row; the key hints must
    /// yield space rather than overwrite the freshness readout.
    fn footer_row(width: u16) -> String {
        let now = Utc::now();
        let app = app_with_providers(now);
        let theme = Theme::new(false);
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| render_footer(frame, Rect::new(0, 0, width, 1), &app, &theme, now))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..width)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect()
    }

    /// The freshness paragraph renders over the key hints, so an overrun
    /// shows up as a hint running straight into "Codex" with no gap
    /// (e.g. "o sCodex fresh"). Every width must keep them separated.
    #[test]
    fn footer_keys_never_collide_with_freshness() {
        for width in [60u16, 80, 100, 110, 120, 140, 160, 200] {
            let row = footer_row(width);
            let freshness_start = row
                .find("Codex fresh")
                .unwrap_or_else(|| panic!("width {width}: freshness missing entirely: {row:?}"));
            // Whatever precedes the freshness block must be blank padding,
            // not a key hint sliced in half.
            assert!(
                row[..freshness_start].ends_with("  ") || freshness_start == 0,
                "width {width}: key hints collide with freshness: {row:?}"
            );
        }
    }

    #[test]
    fn footer_keeps_leading_hints_when_space_allows() {
        let row = footer_row(200);
        assert!(row.contains("codex"), "wide footer lost its hints: {row:?}");
        assert!(row.contains("planner"), "wide footer lost hints: {row:?}");
    }
}

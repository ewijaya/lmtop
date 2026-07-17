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
    let right = Line::from(Span::styled(format!("{clock} "), theme.dim())).right_aligned();
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
            ("Tab", "panel"),
            ("v", "chart"),
            ("Enter", "detail"),
            ("/", "filter"),
            ("?", "help"),
            ("q", "quit"),
        ]
    } else {
        &[
            ("1", "codex"),
            ("2", "claude"),
            ("3", "all"),
            ("4", "planner"),
            ("Tab", "panel"),
            ("v", "chart mode"),
            ("←→", "pan"),
            ("Enter", "detail"),
            ("o", "sort"),
            ("/", "filter"),
            ("r", "refresh"),
            ("p", "pause"),
            ("?", "help"),
            ("q", "quit"),
        ]
    };
    let mut spans = Vec::new();
    for (key, label) in keys {
        spans.push(Span::styled(
            format!(" {key} "),
            theme.title().add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::styled(format!("{label} "), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    // Right side: freshness per provider.
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
    frame.render_widget(Paragraph::new(Line::from(right).right_aligned()), area);
}

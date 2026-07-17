//! Session drill-down overlay: everything known about one session —
//! timing, throughput, context occupancy, token composition, and the
//! per-model split. Opened with Enter on the session table.

use super::bar;
use crate::domain::{ModelIdentity, SessionState, SessionUsage};
use crate::tui::layout::centered;
use crate::tui::theme::{Theme, fmt_age, fmt_tokens};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

pub fn render_session_detail(
    frame: &mut Frame,
    area: Rect,
    session: &SessionUsage,
    theme: &Theme,
    now: DateTime<Utc>,
) {
    let models: Vec<(&String, &crate::domain::TokenCounts)> = {
        let mut v: Vec<_> = session.tokens_by_model.iter().collect();
        v.sort_by_key(|(_, t)| std::cmp::Reverse(t.total()));
        v
    };
    let height = (13 + models.len() as u16).min(area.height);
    let width = 76u16.min(area.width.saturating_sub(2)).max(40);
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);

    let title = session
        .project
        .clone()
        .unwrap_or_else(|| session.id.chars().take(12).collect());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.border_focused())
        .title(Span::styled(
            format!(" SESSION — {title} "),
            Style::default()
                .fg(theme.provider(session.provider))
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(" Enter/Esc close ", theme.dim())).right_aligned(),
        );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();
    let label = |s: &str| Span::styled(format!("{s:<11}"), theme.dim());

    // Identity.
    let state = session.state(now);
    let state_span = match state {
        SessionState::Active => Span::styled("active", Style::default().fg(theme.good())),
        SessionState::Recent => Span::styled("recent", theme.text()),
        SessionState::Idle => Span::styled("idle", theme.dim()),
    };
    let model = session
        .model
        .as_ref()
        .map(|m| m.display.clone())
        .unwrap_or_else(|| "-".into());
    lines.push(Line::from(vec![
        label("session"),
        Span::styled(
            session.provider.display_name(),
            Style::default().fg(theme.provider(session.provider)),
        ),
        Span::styled("  ", theme.dim()),
        Span::styled(model, theme.text()),
        Span::styled("  ", theme.dim()),
        state_span,
        Span::styled(
            format!("  id {}", session.id.chars().take(20).collect::<String>()),
            theme.dim(),
        ),
    ]));

    // Timing.
    let started = session
        .started_at
        .map(|t| format!("{} ago", fmt_age(now.signed_duration_since(t).num_seconds())))
        .unwrap_or_else(|| "-".into());
    let last = session
        .last_activity
        .map(|t| format!("{} ago", fmt_age(now.signed_duration_since(t).num_seconds())))
        .unwrap_or_else(|| "-".into());
    let duration = match (session.started_at, session.last_activity) {
        (Some(s), Some(l)) if l > s => fmt_age(l.signed_duration_since(s).num_seconds()),
        _ => "-".into(),
    };
    lines.push(Line::from(vec![
        label("timing"),
        Span::styled(format!("started {started}"), theme.text()),
        Span::styled("  ·  ", theme.dim()),
        Span::styled(format!("last activity {last}"), theme.text()),
        Span::styled("  ·  ", theme.dim()),
        Span::styled(format!("span {duration}"), theme.text()),
    ]));

    // Throughput.
    let rate = session
        .tokens_per_minute
        .map(|r| format!("{}/min", fmt_tokens(r as u64)))
        .unwrap_or_else(|| "idle".into());
    lines.push(Line::from(vec![
        label("rate"),
        Span::styled(rate, theme.text()),
    ]));

    // Context occupancy.
    let bar_width = (inner.width as usize).saturating_sub(30).clamp(10, 30);
    match session.context_percent() {
        Some(pct) => {
            let used = session.context_tokens.unwrap_or(0);
            let window = session.context_window.unwrap_or(0);
            lines.push(Line::from(vec![
                label("context"),
                Span::styled(
                    bar(bar_width, pct, theme.ascii),
                    Style::default().fg(theme.gauge_color(pct)),
                ),
                Span::styled(
                    format!(" {pct:.0}% ({} of {})", fmt_tokens(used), fmt_tokens(window)),
                    theme.text(),
                ),
            ]));
        }
        None => {
            lines.push(Line::from(vec![
                label("context"),
                Span::styled("not reported", theme.dim()),
            ]));
        }
    }

    // Token composition.
    let t = &session.tokens;
    lines.push(Line::from(vec![
        label("tokens"),
        Span::styled(format!("total {}", fmt_tokens(t.total())), theme.title()),
    ]));
    lines.push(Line::from(vec![
        label(""),
        Span::styled(
            format!(
                "in {}  cached {}  cachew {}  out {}{}",
                fmt_tokens(t.input),
                fmt_tokens(t.cached_input),
                fmt_tokens(t.cache_creation),
                fmt_tokens(t.output),
                if t.reasoning > 0 {
                    format!("  reasoning {}", fmt_tokens(t.reasoning))
                } else {
                    String::new()
                }
            ),
            theme.dim(),
        ),
    ]));
    let input_total = t.total_input();
    if input_total > 0 && t.cached_input > 0 {
        let hit = t.cached_input as f64 / input_total as f64 * 100.0;
        lines.push(Line::from(vec![
            label(""),
            Span::styled(format!("cache hit rate {hit:.0}% of input"), theme.dim()),
        ]));
    }

    // Per-model split.
    if !models.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(label("by model")));
        let grand = t.total().max(1);
        for (raw, tokens) in models {
            let identity = ModelIdentity::normalize(raw);
            let pct = tokens.total() as f64 / grand as f64 * 100.0;
            let color = theme.model(identity.family);
            let mut name = identity.display;
            name.truncate(16);
            lines.push(Line::from(vec![
                Span::styled(format!("  {name:<16}"), Style::default().fg(color)),
                Span::styled(
                    bar(bar_width, pct, theme.ascii),
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!(" {:>6} {pct:>4.0}%", fmt_tokens(tokens.total())),
                    theme.text(),
                ),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

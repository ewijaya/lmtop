use super::bar;
use crate::domain::{
    Capability, Provider, ProviderSnapshot, QuotaOutlook, QuotaWindow, QuotaWindowKind,
    SessionState,
};
use crate::tui::theme::{Theme, fmt_duration_until, fmt_tokens};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

/// Capacity-planning verdict for one quota window: will it run out before
/// the reset? Derived purely from the provider-reported percentage trend.
fn outlook_span<'a>(w: &QuotaWindow, theme: &Theme, now: chrono::DateTime<Utc>) -> Span<'a> {
    let (ok, warn, bad) = if theme.ascii {
        ("ok", "!", "x")
    } else {
        ("✓", "⚠", "✗")
    };
    match w.outlook() {
        QuotaOutlook::Exhausted => Span::styled(
            format!(" {bad} exhausted"),
            Style::default().fg(theme.gauge_color(100.0)),
        ),
        QuotaOutlook::AtRisk {
            projected_exhaustion,
        } => {
            let eta = fmt_duration_until(
                projected_exhaustion
                    .signed_duration_since(now)
                    .num_seconds(),
            );
            Span::styled(
                format!(" {warn} empty ~{eta}{}", confidence_suffix(w)),
                Style::default().fg(theme.gauge_color(95.0)),
            )
        }
        QuotaOutlook::Lasts => Span::styled(
            format!(" {ok} lasts{}", confidence_suffix(w)),
            Style::default().fg(theme.gauge_color(0.0)),
        ),
        QuotaOutlook::Unknown => Span::styled(" · trend n/a".to_string(), theme.dim()),
    }
}

/// Compact confidence marker for a projection, e.g. "·med". Estimates are
/// always labeled with how much the trend can be trusted.
fn confidence_suffix(w: &QuotaWindow) -> String {
    match w.trend_confidence {
        Some(c) => format!("·{}", c.short_label()),
        None => String::new(),
    }
}

/// Quota + token summary panel for one provider. `detailed` adds capability
/// and health lines (used by the single-provider views).
#[allow(clippy::too_many_arguments)]
pub fn render_provider_panel(
    frame: &mut Frame,
    area: Rect,
    provider: Provider,
    snapshot: Option<&ProviderSnapshot>,
    theme: &Theme,
    now: DateTime<Utc>,
    focused: bool,
    detailed: bool,
) {
    let border_style = if focused {
        theme.border_focused()
    } else {
        theme.border()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(border_style)
        .title(Span::styled(
            format!(" {} ", provider.display_name().to_uppercase()),
            Style::default()
                .fg(theme.provider(provider))
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(snap) = snapshot else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("starting…", theme.dim()))),
            inner,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    let bar_width = (inner.width as usize).saturating_sub(44).clamp(6, 20);

    // Provider quota windows: authoritative provider percentages only.
    if snap.supports(Capability::ProviderQuota) && !snap.quota_windows.is_empty() {
        // Show 5h then weekly then any unknown windows, by kind not order.
        let mut windows: Vec<_> = snap.quota_windows.iter().collect();
        windows.sort_by_key(|w| match w.kind {
            QuotaWindowKind::FiveHour => 0,
            QuotaWindowKind::Weekly => 1,
            QuotaWindowKind::Unknown => 2,
        });
        for w in windows {
            let pct = w.used_percent;
            let reset = w
                .resets_at
                .map(|t| {
                    format!(
                        " ↻{}",
                        fmt_duration_until(t.signed_duration_since(now).num_seconds())
                    )
                })
                .unwrap_or_default();
            let reset = if theme.ascii {
                reset.replace('↻', "r ")
            } else {
                reset
            };
            let mut spans = vec![
                Span::styled(format!("{:<8}", w.label()), theme.text()),
                Span::styled(
                    bar(bar_width, pct, theme.ascii),
                    Style::default().fg(theme.gauge_color(pct)),
                ),
                Span::styled(format!(" {pct:>5.1}%"), theme.text()),
                Span::styled(reset, theme.dim()),
            ];
            spans.push(outlook_span(w, theme, now));
            // Flag data that is only as fresh as the last provider event.
            let age_secs = now.signed_duration_since(w.captured_at).num_seconds();
            if age_secs > 600 {
                spans.push(Span::styled(
                    format!(" @{}", crate::tui::theme::fmt_age(age_secs)),
                    theme.dim(),
                ));
            }
            lines.push(Line::from(spans));
        }
    } else {
        let reason = if snap.supports(Capability::ProviderQuota) {
            "no recent provider data"
        } else {
            "not exposed locally"
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "Quota"), theme.text()),
            Span::styled(format!("unavailable ({reason})"), theme.dim()),
        ]));
    }

    // Credits.
    if let Some(credits) = &snap.credits {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "Credits"), theme.text()),
            Span::styled(format!("{:.0}", credits.balance), theme.text()),
        ]));
    } else if snap.supports(Capability::Credits) {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "Credits"), theme.text()),
            Span::styled("unavailable", theme.dim()),
        ]));
    }

    // Observed tokens: session and calendar week. Explicitly labeled as
    // observed so they are never mistaken for quota consumption.
    let session_tokens = snap.current_session_tokens.total();
    let session_label = if session_tokens > 0 {
        fmt_tokens(session_tokens)
    } else {
        "idle".to_string()
    };
    let week_tokens = snap.week.as_ref().map(|w| w.tokens.total()).unwrap_or(0);
    lines.push(Line::from(vec![
        Span::styled(format!("{:<12}", "Observed"), theme.text()),
        Span::styled(
            format!("session {session_label}"),
            Style::default().fg(theme.provider(provider)),
        ),
        Span::styled("  ·  ", theme.dim()),
        Span::styled(
            format!("week {}", fmt_tokens(week_tokens)),
            Style::default().fg(theme.provider(provider)),
        ),
    ]));

    // Active sessions + last model.
    let active = snap
        .sessions
        .iter()
        .filter(|s| s.state(now) == SessionState::Active)
        .count();
    let model = snap
        .sessions
        .first()
        .and_then(|s| s.model.as_ref())
        .map(|m| m.display.clone())
        .unwrap_or_else(|| "-".into());
    lines.push(Line::from(vec![
        Span::styled(format!("{:<12}", "Sessions"), theme.text()),
        Span::styled(format!("{active} active"), theme.text()),
        Span::styled("  ·  ", theme.dim()),
        Span::styled(format!("model {model}"), theme.dim()),
    ]));

    if detailed {
        lines.push(Line::from(Span::styled(
            format!(
                "health: {} · files {} · parse errors {}",
                theme.status(snap.health.status).1,
                snap.health.files_scanned,
                snap.health.parse_errors
            ),
            theme.dim(),
        )));
        if let Some(msg) = &snap.health.message {
            lines.push(Line::from(Span::styled(msg.clone(), theme.dim())));
        }
        let caps = Capability::ALL
            .iter()
            .map(|c| {
                if snap.supports(*c) {
                    format!("+{}", c.label())
                } else {
                    format!("-{}", c.label())
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(Line::from(Span::styled(caps, theme.dim())));
    } else if let Some(msg) = &snap.health.message
        && snap.health.status != crate::domain::CollectorStatus::Ok
    {
        lines.push(Line::from(Span::styled(msg.clone(), theme.dim())));
    }

    frame.render_widget(
        Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true }),
        inner,
    );
}

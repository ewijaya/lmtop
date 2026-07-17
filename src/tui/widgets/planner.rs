//! Planner view: for each provider, capacity-vs-time race bars per quota
//! window, burn scenarios, sustainable-burn guidance, and calendar-week
//! pacing. Answers the product's core question — will it last? — with
//! room to breathe.

use super::bar;
use super::provider_panel::pad_label;
use crate::domain::{
    Capability, Provider, ProviderSnapshot, QuotaOutlook, QuotaWindow, QuotaWindowKind,
};
use crate::persist::HistoryStore;
use crate::tui::theme::{Theme, fmt_duration_until, fmt_tokens, sparkline};
use chrono::{DateTime, Duration, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

/// Fraction of the window elapsed, when its duration and reset are known.
fn elapsed_percent(w: &QuotaWindow, now: DateTime<Utc>) -> Option<f64> {
    let resets_at = w.resets_at?;
    let minutes = w.window_minutes? as i64;
    if minutes <= 0 {
        return None;
    }
    let start = resets_at - Duration::minutes(minutes);
    let elapsed = now.signed_duration_since(start).num_seconds() as f64;
    Some((elapsed / (minutes * 60) as f64 * 100.0).clamp(0.0, 100.0))
}

pub fn render_planner(
    frame: &mut Frame,
    area: Rect,
    provider: Provider,
    snapshot: Option<&ProviderSnapshot>,
    history: Option<&HistoryStore>,
    theme: &Theme,
    now: DateTime<Utc>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.border())
        .title(Span::styled(
            format!(" {} — PLANNER ", provider.display_name().to_uppercase()),
            Style::default()
                .fg(theme.provider(provider))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(snap) = snapshot else {
        frame.render_widget(
            Paragraph::new(Span::styled("starting…", theme.dim())),
            inner,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    let bar_width = (inner.width as usize).saturating_sub(30).clamp(10, 40);

    let mut windows: Vec<&QuotaWindow> = snap.quota_windows.iter().collect();
    windows.sort_by_key(|w| match w.kind {
        QuotaWindowKind::FiveHour => 0,
        QuotaWindowKind::Weekly => 1,
        QuotaWindowKind::Unknown => 2,
    });

    if windows.is_empty() {
        let reason = if snap.supports(Capability::ProviderQuota) {
            "no recent provider quota data — try --live"
        } else {
            "provider quota not exposed"
        };
        lines.push(Line::from(Span::styled(reason, theme.dim())));
    }

    for w in windows {
        // Continuation rows indent to this window's (possibly padded-out)
        // label width so long labels like "Weekly (Fable)" stay aligned.
        let indent = " ".repeat(pad_label(&w.label(), 10).chars().count());
        if w.is_expired(now) {
            lines.push(Line::from(vec![
                Span::styled(pad_label(&w.label(), 10), theme.title()),
                Span::styled(
                    format!("stale — window already reset (last {:.0}%)", w.used_percent),
                    theme.dim(),
                ),
            ]));
            lines.push(Line::default());
            continue;
        }
        // Race bars: capacity used vs time elapsed. Capacity winning the
        // race is the whole story of "will it last".
        lines.push(Line::from(vec![
            Span::styled(pad_label(&w.label(), 10), theme.title()),
            Span::styled("capacity ", theme.dim()),
            Span::styled(
                bar(bar_width, w.used_percent, theme.ascii),
                Style::default().fg(theme.gauge_color(w.used_percent)),
            ),
            Span::styled(format!(" {:>5.1}%", w.used_percent), theme.text()),
        ]));
        match elapsed_percent(w, now) {
            Some(elapsed) => {
                let reset_in = w
                    .resets_at
                    .map(|t| {
                        format!(
                            " resets {}",
                            fmt_duration_until(t.signed_duration_since(now).num_seconds())
                        )
                    })
                    .unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled(indent.clone(), theme.text()),
                    Span::styled("time     ", theme.dim()),
                    Span::styled(
                        bar(bar_width, elapsed, theme.ascii),
                        Style::default().fg(theme.provider_dim(provider)),
                    ),
                    Span::styled(format!(" {elapsed:>5.1}%"), theme.dim()),
                    Span::styled(reset_in, theme.dim()),
                ]));
                let pace = w.used_percent - elapsed;
                let (pace_text, pace_color) = if pace <= -5.0 {
                    ("under budget — capacity to spare".to_string(), theme.good())
                } else if pace < 5.0 {
                    ("on pace with the window".to_string(), theme.warn())
                } else {
                    (
                        format!("{pace:.0}pp ahead of the clock — running hot"),
                        theme.bad(),
                    )
                };
                lines.push(Line::from(vec![
                    Span::styled(indent.clone(), theme.text()),
                    Span::styled("pace     ", theme.dim()),
                    Span::styled(pace_text, Style::default().fg(pace_color)),
                ]));
            }
            None => {
                lines.push(Line::from(vec![
                    Span::styled(indent.clone(), theme.text()),
                    Span::styled("time     unknown (no reset reported)", theme.dim()),
                ]));
            }
        }

        // Burn scenario line: current burn, verdict, sustainable ceiling.
        let mut spans = vec![Span::styled(indent.clone(), theme.text())];
        match (w.burn_per_hour, w.resets_at) {
            (Some(burn), Some(reset)) => {
                let hours_left =
                    reset.signed_duration_since(now).num_seconds().max(0) as f64 / 3600.0;
                let sustainable = if hours_left > 0.05 {
                    (100.0 - w.used_percent).max(0.0) / hours_left
                } else {
                    f64::INFINITY
                };
                let confidence = w
                    .trend_confidence
                    .map(|c| format!(" ({})", c.short_label()))
                    .unwrap_or_default();
                spans.push(Span::styled(
                    format!("burn {burn:.1}%/h{confidence}"),
                    theme.text(),
                ));
                spans.push(Span::styled(
                    format!(" · sustainable ≤ {sustainable:.1}%/h · "),
                    theme.dim(),
                ));
                match w.outlook() {
                    QuotaOutlook::Exhausted => {
                        spans.push(Span::styled("✗ exhausted", Style::default().fg(theme.bad())));
                    }
                    QuotaOutlook::AtRisk {
                        projected_exhaustion,
                    } => {
                        let empty_in = fmt_duration_until(
                            projected_exhaustion
                                .signed_duration_since(now)
                                .num_seconds(),
                        );
                        let short_by = fmt_duration_until(
                            reset
                                .signed_duration_since(projected_exhaustion)
                                .num_seconds(),
                        );
                        spans.push(Span::styled(
                            format!("⚠ empty ~{empty_in} ({short_by} before reset)"),
                            Style::default().fg(theme.bad()),
                        ));
                    }
                    QuotaOutlook::Lasts => {
                        let at_reset = (w.used_percent
                            + burn * hours_left)
                            .clamp(w.used_percent, 100.0);
                        spans.push(Span::styled(
                            format!("✓ lasts — ~{at_reset:.0}% at reset"),
                            Style::default().fg(theme.good()),
                        ));
                    }
                    QuotaOutlook::Unknown => {
                        spans.push(Span::styled("trend n/a", theme.dim()));
                    }
                }
            }
            _ => {
                spans.push(Span::styled(
                    "burn trend n/a — appears as lmtop watches the window move",
                    theme.dim(),
                ));
            }
        }
        lines.push(Line::from(spans));

        // Quota trend sparkline from persisted samples (last 6 hours).
        if let Some(history) = history {
            let points: Vec<f64> = history
                .quota_series(snap.provider, now - Duration::hours(6), now)
                .into_iter()
                .filter(|q| q.kind == w.kind && q.scope == w.scope)
                .map(|q| q.used_percent)
                .collect();
            if let Some(spark) = sparkline(&points, bar_width.min(30), theme.ascii) {
                lines.push(Line::from(vec![
                    Span::styled(indent.clone(), theme.text()),
                    Span::styled("trend 6h ", theme.dim()),
                    Span::styled(spark, Style::default().fg(theme.provider(provider))),
                ]));
            }
        }
        lines.push(Line::default());
    }

    // Calendar-week pacing: observed tokens only, and said so. Never a
    // quota estimate — providers weight tokens in unpublished ways.
    if let Some(week) = &snap.week {
        let span = week
            .week_end
            .signed_duration_since(week.week_start)
            .num_seconds() as f64;
        let elapsed = now.signed_duration_since(week.week_start).num_seconds() as f64;
        let elapsed_pct = (elapsed / span.max(1.0) * 100.0).clamp(0.0, 100.0);
        lines.push(Line::from(vec![
            Span::styled(format!("{:<10}", "Cal week"), theme.title()),
            Span::styled("elapsed  ", theme.dim()),
            Span::styled(
                bar(bar_width, elapsed_pct, theme.ascii),
                Style::default().fg(theme.provider_dim(provider)),
            ),
            Span::styled(format!(" {elapsed_pct:>5.1}%"), theme.dim()),
            Span::styled(
                format!(
                    " · observed {} across {} sessions",
                    fmt_tokens(week.tokens.total()),
                    week.sessions
                ),
                theme.text(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("{:<10}", ""), theme.text()),
            Span::styled(
                "observed tokens ≠ quota consumption (providers weight models & caching)",
                theme.dim(),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

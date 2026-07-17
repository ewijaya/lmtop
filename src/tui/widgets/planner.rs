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

/// A window this far through is effectively over: with almost no time
/// left, pace advice is vacuous ("capacity to spare" is trivially true at
/// 99% elapsed) and would only mislead.
const WINDOW_ENDING_PERCENT: f64 = 92.0;

/// How the spend compares to the clock. Kept separate from rendering so
/// the wording can be tested directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pace {
    /// The window is nearly over; what is left is about to be returned.
    Ending,
    Under,
    Steady,
    Hot,
}

fn pace_of(used_percent: f64, elapsed_percent: f64) -> Pace {
    if elapsed_percent >= WINDOW_ENDING_PERCENT {
        return Pace::Ending;
    }
    let delta = used_percent - elapsed_percent;
    if delta <= -5.0 {
        Pace::Under
    } else if delta < 5.0 {
        Pace::Steady
    } else {
        Pace::Hot
    }
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
                let delta = w.used_percent - elapsed;
                let (pace_text, pace_color) = match pace_of(w.used_percent, elapsed) {
                    // Near the reset, "capacity to spare" is trivially
                    // true and tells you nothing worth acting on; the
                    // useful fact is that the window is about to refill.
                    Pace::Ending => (
                        format!(
                            "window ending — {:.0}% unused, refills at reset",
                            (100.0 - w.used_percent).max(0.0)
                        ),
                        theme.dim().fg.unwrap_or(theme.good()),
                    ),
                    Pace::Under => ("under budget — capacity to spare".to_string(), theme.good()),
                    Pace::Steady => ("on pace with the window".to_string(), theme.warn()),
                    Pace::Hot => (
                        format!("{delta:.0}pp ahead of the clock — running hot"),
                        theme.bad(),
                    ),
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
                        spans.push(Span::styled(
                            "✗ exhausted",
                            Style::default().fg(theme.bad()),
                        ));
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
                        let at_reset =
                            (w.used_percent + burn * hours_left).clamp(w.used_percent, 100.0);
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

    // Ratatui clips silently, which would leave a window rendered
    // half-way — a capacity bar with no time or pace beneath it reads as
    // missing data rather than a short panel. Trim deliberately and say
    // what was dropped.
    let height = inner.height as usize;
    if lines.len() > height && height > 0 {
        lines.truncate(height.saturating_sub(1));
        lines.push(Line::from(Span::styled(
            "… panel too short to show every window — resize or press 1/2",
            theme.dim(),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CollectorHealth, CollectorStatus};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn pace_reports_a_nearly_finished_window_as_ending() {
        // The real case from a screenshot: 56% used with 10 minutes left of
        // a 5h window. "Capacity to spare" is true but useless there.
        assert_eq!(pace_of(56.0, 96.5), Pace::Ending);
        assert_eq!(pace_of(5.0, 99.0), Pace::Ending);
        // An exhausted-but-ending window is still ending, not "hot".
        assert_eq!(pace_of(99.0, 98.0), Pace::Ending);
    }

    #[test]
    fn pace_classifies_mid_window_spend() {
        assert_eq!(pace_of(20.0, 50.0), Pace::Under);
        assert_eq!(pace_of(48.0, 50.0), Pace::Steady);
        assert_eq!(pace_of(80.0, 50.0), Pace::Hot);
        // Boundaries: the window must still have room left to advise on.
        assert_eq!(pace_of(50.0, 91.9), Pace::Under);
        assert_eq!(pace_of(50.0, 92.0), Pace::Ending);
    }

    fn window(
        kind: QuotaWindowKind,
        used: f64,
        now: DateTime<Utc>,
        scope: Option<&str>,
    ) -> QuotaWindow {
        QuotaWindow {
            kind,
            used_percent: used,
            window_minutes: Some(300),
            resets_at: Some(now + Duration::hours(2)),
            captured_at: now,
            scope: scope.map(str::to_string),
            burn_per_hour: None,
            projected_exhaustion: None,
            trend_confidence: None,
        }
    }

    fn render_at(height: u16, windows: Vec<QuotaWindow>, now: DateTime<Utc>) -> Vec<String> {
        let mut snap = ProviderSnapshot::empty(
            Provider::Claude,
            CollectorHealth {
                status: CollectorStatus::Ok,
                message: None,
                last_scan: Some(now),
                files_scanned: 1,
                parse_errors: 0,
            },
        );
        snap.quota_windows = windows;
        let theme = Theme::new(false);
        let mut terminal = Terminal::new(TestBackend::new(100, height)).unwrap();
        terminal
            .draw(|frame| {
                render_planner(
                    frame,
                    Rect::new(0, 0, 100, height),
                    Provider::Claude,
                    Some(&snap),
                    None,
                    &theme,
                    now,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..100)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    /// A panel too short to hold every window must say so rather than
    /// leave the last window rendered half-way.
    #[test]
    fn short_panel_flags_dropped_windows() {
        let now = Utc::now();
        let windows = vec![
            window(QuotaWindowKind::FiveHour, 56.0, now, None),
            window(QuotaWindowKind::Weekly, 16.0, now, None),
            window(QuotaWindowKind::Weekly, 24.0, now, Some("Fable")),
        ];
        let rows = render_at(10, windows.clone(), now);
        let text = rows.join("\n");
        assert!(
            text.contains("panel too short"),
            "truncation went unannounced: {text}"
        );

        // Given room, every window renders and no warning appears.
        let rows = render_at(30, windows, now);
        let text = rows.join("\n");
        assert!(
            !text.contains("panel too short"),
            "spurious warning: {text}"
        );
        assert!(text.contains("Fable"), "Fable window missing: {text}");
    }
}

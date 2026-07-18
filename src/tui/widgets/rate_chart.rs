//! The chart panel: observed token rate or the provider-reported quota
//! timeline (sawtooth), over a pannable/zoomable window backed by persisted
//! history. The live snapshot fills the most recent minutes.

use crate::app::{App, ChartMode};
use crate::domain::{Provider, ProviderSnapshot, QuotaWindowKind};
use crate::tui::theme::{Theme, fmt_tokens};
use chrono::{DateTime, Duration, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, BorderType, Borders, Chart, Dataset, GraphType};
use std::collections::BTreeMap;

/// Cap on plotted points per series; wider windows are bucketed down.
const MAX_POINTS: i64 = 720;

pub fn render_chart(
    frame: &mut Frame,
    area: Rect,
    providers: &[(Provider, Option<&ProviderSnapshot>)],
    app: &App,
    theme: &Theme,
    now: DateTime<Utc>,
    focused: bool,
) {
    let border_style = if focused {
        theme.border_focused()
    } else {
        theme.border()
    };
    let end = now - Duration::minutes(app.pan_minutes);
    let start = end - Duration::minutes(app.zoom_minutes);

    let range_label = if app.pan_minutes > 0 {
        format!(
            " ⟨{} window · ends {} ago⟩ ",
            crate::tui::theme::fmt_age(app.zoom_minutes * 60),
            crate::tui::theme::fmt_age(app.pan_minutes * 60)
        )
    } else if app.zoom_minutes != 60 {
        format!(
            " ⟨{} window⟩ ",
            crate::tui::theme::fmt_age(app.zoom_minutes * 60)
        )
    } else {
        " ".into()
    };
    let title_text = match app.chart_mode {
        ChartMode::Rate => format!(" TOKEN RATE (tokens/min, observed){range_label}"),
        ChartMode::Quota => format!(" QUOTA TIMELINE (%, provider-reported){range_label}"),
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(border_style)
        .title(Span::styled(title_text, theme.title()));
    if focused {
        block = block.title_bottom(
            ratatui::text::Line::from(Span::styled(
                " ←/→ pan  +/- zoom  v mode  0 reset ",
                theme.dim(),
            ))
            .right_aligned(),
        );
    }

    match app.chart_mode {
        ChartMode::Rate => {
            render_rate(frame, area, block, providers, app, theme, start, end);
        }
        ChartMode::Quota => {
            render_quota(frame, area, block, providers, app, theme, start, end, now);
        }
    }
}

/// One plotted series: (color, name, points).
type Series = (ratatui::style::Color, String, Vec<(f64, f64)>);

#[allow(clippy::too_many_arguments)]
fn render_rate(
    frame: &mut Frame,
    area: Rect,
    block: Block,
    providers: &[(Provider, Option<&ProviderSnapshot>)],
    app: &App,
    theme: &Theme,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) {
    let bucket_minutes = (app.zoom_minutes / MAX_POINTS).max(1);
    let mut series: Vec<Series> = Vec::new();
    let mut max_y: f64 = 0.0;

    for (provider, snap) in providers {
        // Persisted minutes first, live snapshot samples overlaid on top
        // (the live view has the in-progress minute; both agree elsewhere).
        let mut minutes: BTreeMap<DateTime<Utc>, (u64, u64)> = BTreeMap::new();
        if let Some(history) = &app.history {
            for s in history.rate_series(*provider, start, end) {
                minutes.insert(s.at, (s.input_tokens, s.output_tokens));
            }
        }
        if let Some(snap) = snap {
            for s in &snap.history {
                if s.at >= start && s.at < end {
                    minutes.insert(s.at, (s.input_tokens, s.output_tokens));
                }
            }
        }
        if minutes.is_empty() {
            continue;
        }
        // Bucket into fixed intervals; per-bucket average keeps the y axis
        // in tokens/min at every zoom level.
        let mut in_pts: Vec<(f64, f64)> = Vec::new();
        let mut out_pts: Vec<(f64, f64)> = Vec::new();
        let mut bucket: BTreeMap<i64, (u64, u64)> = BTreeMap::new();
        for (at, (i, o)) in &minutes {
            let offset = end.signed_duration_since(*at).num_minutes() / bucket_minutes;
            let e = bucket.entry(offset).or_default();
            e.0 += i;
            e.1 += o;
        }
        for (offset, (i, o)) in bucket {
            let x = -(offset * bucket_minutes) as f64;
            let i = i as f64 / bucket_minutes as f64;
            let o = o as f64 / bucket_minutes as f64;
            max_y = max_y.max(i).max(o);
            in_pts.push((x, i));
            out_pts.push((x, o));
        }
        in_pts.reverse();
        out_pts.reverse();
        series.push((
            theme.provider_dim(*provider),
            format!("{} in", provider.display_name()),
            in_pts,
        ));
        series.push((
            theme.provider(*provider),
            format!("{} out", provider.display_name()),
            out_pts,
        ));
    }

    if series.is_empty() || max_y <= 0.0 {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                "no activity in this window",
                theme.dim(),
            )),
            inner,
        );
        return;
    }

    let y_max = max_y * 1.1;
    let x_min = -(app.zoom_minutes as f64);
    let chart = Chart::new(build_datasets(&series, theme))
        .x_axis(
            Axis::default()
                .bounds([x_min, 0.0])
                .labels(x_labels(app, theme))
                .style(theme.dim()),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, y_max])
                .labels([
                    Span::styled("0", theme.dim()),
                    Span::styled(fmt_tokens((y_max / 2.0) as u64), theme.dim()),
                    Span::styled(fmt_tokens(y_max as u64), theme.dim()),
                ])
                .style(theme.dim()),
        )
        .block(block);
    frame.render_widget(chart, area);
}

#[allow(clippy::too_many_arguments)]
fn render_quota(
    frame: &mut Frame,
    area: Rect,
    block: Block,
    providers: &[(Provider, Option<&ProviderSnapshot>)],
    app: &App,
    theme: &Theme,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    now: DateTime<Utc>,
) {
    let Some(history) = &app.history else {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                "quota timeline needs persisted history (history.persist = true)",
                theme.dim(),
            )),
            inner,
        );
        return;
    };

    let mut series: Vec<Series> = Vec::new();
    for (provider, snap) in providers {
        // Group persisted points per window identity.
        let mut grouped: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
        for q in history.quota_series(*provider, start, end) {
            let key = format!("{}|{}", q.kind.label(), q.scope.clone().unwrap_or_default());
            let x = -end.signed_duration_since(q.at).num_seconds() as f64 / 60.0;
            grouped.entry(key).or_default().push((x, q.used_percent));
        }
        // The live snapshot's current values anchor the right edge.
        if let Some(snap) = snap {
            for w in &snap.quota_windows {
                if w.captured_at >= start && w.captured_at < end && !w.is_expired(now) {
                    let key = format!("{}|{}", w.kind.label(), w.scope.clone().unwrap_or_default());
                    let x = -end.signed_duration_since(w.captured_at).num_seconds() as f64 / 60.0;
                    let pts = grouped.entry(key).or_default();
                    if pts.last().is_none_or(|p| p.0 < x) {
                        pts.push((x, w.used_percent));
                    }
                }
            }
        }
        for (key, pts) in grouped {
            if pts.len() < 2 {
                continue; // a single dot renders as noise
            }
            let is_five_hour = key.starts_with(QuotaWindowKind::FiveHour.label());
            let color = if is_five_hour {
                theme.provider(*provider)
            } else {
                theme.provider_dim(*provider)
            };
            let label = key.trim_end_matches('|').replace('|', " ");
            series.push((color, format!("{} {label}", provider.display_name()), pts));
        }
    }

    if series.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                "no quota samples in this window yet — they accumulate as lmtop runs",
                theme.dim(),
            )),
            inner,
        );
        return;
    }

    let x_min = -(app.zoom_minutes as f64);
    let chart = Chart::new(build_datasets(&series, theme))
        .x_axis(
            Axis::default()
                .bounds([x_min, 0.0])
                .labels(x_labels(app, theme))
                .style(theme.dim()),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, 100.0])
                .labels([
                    Span::styled("0%", theme.dim()),
                    Span::styled("50%", theme.dim()),
                    Span::styled("100%", theme.dim()),
                ])
                .style(theme.dim()),
        )
        .block(block);
    frame.render_widget(chart, area);
}

fn build_datasets<'a>(series: &'a [Series], theme: &Theme) -> Vec<Dataset<'a>> {
    let marker = theme.marker();
    series
        .iter()
        .map(|(color, name, pts)| {
            Dataset::default()
                .name(name.clone())
                .marker(marker)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(pts)
        })
        .collect()
}

fn x_labels(app: &App, theme: &Theme) -> [Span<'static>; 2] {
    let left = crate::tui::theme::fmt_age((app.pan_minutes + app.zoom_minutes) * 60);
    let right = if app.pan_minutes > 0 {
        format!("{} ago", crate::tui::theme::fmt_age(app.pan_minutes * 60))
    } else {
        "now".to_string()
    };
    [
        Span::styled(format!("{left} ago"), theme.dim()),
        Span::styled(right, theme.dim()),
    ]
}

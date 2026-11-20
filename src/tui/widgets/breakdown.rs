use super::bar;
use crate::domain::{ModelWeekUsage, Provider, ProviderSnapshot};
use crate::tui::theme::{Theme, fmt_tokens};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

fn panel_block<'a>(title: &'a str, theme: &Theme, focused: bool) -> Block<'a> {
    let border_style = if focused {
        theme.border_focused()
    } else {
        theme.border()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(border_style)
        .title(Span::styled(format!(" {title} "), theme.title()))
}

/// Weekly observed-token totals per provider (calendar week, local time).
pub fn render_weekly(
    frame: &mut Frame,
    area: Rect,
    providers: &[(Provider, Option<&ProviderSnapshot>)],
    theme: &Theme,
    focused: bool,
) {
    let block = panel_block("WEEKLY USAGE (observed)", theme, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let compact = inner.height as usize <= 2 * providers.len();
    let mut week_range: Option<String> = None;
    for (provider, snap) in providers {
        let Some(snap) = snap else { continue };
        let Some(week) = &snap.week else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<9}", provider.display_name()),
                    Style::default().fg(theme.provider(*provider)),
                ),
                Span::styled("unavailable", theme.dim()),
            ]));
            continue;
        };
        if week_range.is_none() {
            let start = week.week_start.with_timezone(&chrono::Local);
            let end = (week.week_end - chrono::Duration::seconds(1)).with_timezone(&chrono::Local);
            week_range = Some(format!(
                "{} – {}",
                start.format("%b %d"),
                end.format("%b %d")
            ));
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<9}", provider.display_name()),
                Style::default()
                    .fg(theme.provider(*provider))
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(
                format!("total {:<8}", fmt_tokens(week.tokens.total())),
                theme.text(),
            ),
            Span::styled(format!("({} sessions)", week.sessions), theme.dim()),
        ]));
        if compact {
            continue;
        }
        let unattr = if week.tokens.unattributed > 0 {
            format!("  unattr {}", fmt_tokens(week.tokens.unattributed))
        } else {
            String::new()
        };
        lines.push(Line::from(Span::styled(
            format!(
                "  in {}  cached {}  cachew {}  out {}{unattr}",
                fmt_tokens(week.tokens.input),
                fmt_tokens(week.tokens.cached_input),
                fmt_tokens(week.tokens.cache_creation),
                fmt_tokens(week.tokens.output),
            ),
            theme.dim(),
        )));
    }
    if let Some(range) = week_range {
        lines.push(Line::from(Span::styled(
            format!("week {range} (local)"),
            theme.dim(),
        )));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("no data", theme.dim())));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Per-model weekly breakdown across the given providers, largest first.
pub fn render_breakdown(
    frame: &mut Frame,
    area: Rect,
    providers: &[(Provider, Option<&ProviderSnapshot>)],
    theme: &Theme,
    focused: bool,
) {
    let block = panel_block("MODEL BREAKDOWN (week)", theme, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut rows: Vec<(Provider, &ModelWeekUsage)> = Vec::new();
    let mut grand_total: u128 = 0;
    for (provider, snap) in providers {
        let Some(week) = snap.as_ref().and_then(|s| s.week.as_ref()) else {
            continue;
        };
        for usage in week.by_model.values() {
            grand_total += u128::from(usage.tokens.total());
            rows.push((*provider, usage));
        }
    }
    rows.sort_by_key(|(_, usage)| std::cmp::Reverse(usage.tokens.total()));

    if rows.is_empty() || grand_total == 0 {
        frame.render_widget(
            Paragraph::new(Span::styled("no usage this week", theme.dim())),
            inner,
        );
        return;
    }

    let name_width = if inner.width >= 43 { 25 } else { 14 };
    let bar_width = (inner.width as usize)
        .saturating_sub(name_width + 18)
        .clamp(5, 20);
    let lines: Vec<Line> = rows
        .iter()
        .take(inner.height as usize)
        .map(|(_, usage)| {
            let pct = usage.tokens.total() as f64 / grand_total as f64 * 100.0;
            let color = theme.model(usage.model.family);
            let display = if name_width < 25 && usage.model.display == "auxiliary (model unknown)" {
                "aux (unknown)"
            } else {
                &usage.model.display
            };
            let name: String = display.chars().take(name_width).collect();
            Line::from(vec![
                Span::styled(format!("{name:<name_width$}"), Style::default().fg(color)),
                Span::styled(bar(bar_width, pct, theme.ascii), Style::default().fg(color)),
                Span::styled(
                    format!(" {:>6} {pct:>4.0}%", fmt_tokens(usage.tokens.total())),
                    theme.text(),
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

use crate::domain::{Provider, ProviderSnapshot};
use crate::tui::theme::{Theme, fmt_tokens};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::symbols;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, BorderType, Borders, Chart, Dataset, GraphType};

/// One plotted series: (provider, is_input, points).
type RateSeries = (Provider, bool, Vec<(f64, f64)>);

/// Token-rate chart: one input and one output series per provider,
/// per-minute buckets over the shared history window.
pub fn render_rate_chart(
    frame: &mut Frame,
    area: Rect,
    providers: &[(Provider, Option<&ProviderSnapshot>)],
    theme: &Theme,
    focused: bool,
) {
    let border_style = if focused {
        theme.border_focused()
    } else {
        theme.border()
    };

    // Build (minutes-ago, tokens/min) series per provider and direction.
    // Series are captured as owned vecs first; Dataset borrows them.
    let mut series: Vec<RateSeries> = Vec::new();
    let mut max_y: f64 = 0.0;
    let mut window_minutes: f64 = 0.0;
    for (provider, snap) in providers {
        let Some(snap) = snap else { continue };
        let n = snap.history.len();
        if n == 0 {
            continue;
        }
        window_minutes = window_minutes.max(n as f64);
        let mut input_pts = Vec::with_capacity(n);
        let mut output_pts = Vec::with_capacity(n);
        for (i, sample) in snap.history.iter().enumerate() {
            // X axis: minutes ago, oldest on the left (negative to 0).
            let x = -((n - 1 - i) as f64);
            input_pts.push((x, sample.input_tokens as f64));
            output_pts.push((x, sample.output_tokens as f64));
            max_y = max_y
                .max(sample.input_tokens as f64)
                .max(sample.output_tokens as f64);
        }
        series.push((*provider, true, input_pts));
        series.push((*provider, false, output_pts));
    }

    let title = Span::styled(" TOKEN RATE (tokens/min, observed) ", theme.title());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(border_style)
        .title(title);

    if series.is_empty() || max_y <= 0.0 {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled("no recent activity", theme.dim())),
            inner,
        );
        return;
    }

    let marker = if theme.ascii {
        symbols::Marker::Dot
    } else {
        symbols::Marker::Braille
    };
    let datasets: Vec<Dataset> = series
        .iter()
        .map(|(provider, is_input, pts)| {
            let color = if *is_input {
                theme.provider_dim(*provider)
            } else {
                theme.provider(*provider)
            };
            let name = format!(
                "{} {}",
                provider.display_name(),
                if *is_input { "in" } else { "out" }
            );
            Dataset::default()
                .name(name)
                .marker(marker)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(color))
                .data(pts)
        })
        .collect();

    let y_max = max_y * 1.1;
    let x_min = -(window_minutes - 1.0).max(1.0);
    let chart = Chart::new(datasets)
        .x_axis(
            Axis::default()
                .bounds([x_min, 0.0])
                .labels([
                    Span::styled(format!("{:.0}m ago", -x_min), theme.dim()),
                    Span::styled("now", theme.dim()),
                ])
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

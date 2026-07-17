use crate::domain::{SessionState, SessionUsage};
use crate::tui::theme::{Theme, fmt_age, fmt_tokens};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Cell, Row, Table};

#[allow(clippy::too_many_arguments)]
pub fn render_sessions(
    frame: &mut Frame,
    area: Rect,
    sessions: &[&SessionUsage],
    theme: &Theme,
    now: DateTime<Utc>,
    focused: bool,
    scroll: usize,
    narrow: bool,
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
            format!(" SESSIONS ({}) ", sessions.len()),
            theme.title(),
        ));
    let inner = block.inner(area);

    let visible_rows = inner.height.saturating_sub(1) as usize; // minus header
    let max_scroll = sessions.len().saturating_sub(visible_rows.max(1));
    let scroll = scroll.min(max_scroll);

    let header_cells: Vec<&str> = if narrow {
        vec!["Prov", "Model", "Tokens", "Age"]
    } else {
        vec![
            "Provider", "Model", "Context", "Tok/min", "Tokens", "Project", "Age", "State",
        ]
    };
    let header = Row::new(
        header_cells
            .into_iter()
            .map(|h| Cell::from(Span::styled(h, theme.dim()))),
    );

    let rows: Vec<Row> = sessions
        .iter()
        .skip(scroll)
        .take(visible_rows)
        .map(|s| {
            let state = s.state(now);
            let state_style = match state {
                SessionState::Active => Style::default().fg(theme.gauge_color(0.0)),
                SessionState::Recent => theme.text(),
                SessionState::Idle => theme.dim(),
            };
            let provider_style = Style::default().fg(theme.provider(s.provider));
            let model = s
                .model
                .as_ref()
                .map(|m| m.display.clone())
                .unwrap_or_else(|| "-".into());
            let age = s
                .last_activity
                .map(|t| fmt_age(now.signed_duration_since(t).num_seconds()))
                .unwrap_or_else(|| "-".into());
            let tokens = fmt_tokens(s.tokens.total());
            if narrow {
                Row::new(vec![
                    Cell::from(Span::styled(s.provider.display_name(), provider_style)),
                    Cell::from(Span::styled(model, state_style)),
                    Cell::from(Span::styled(tokens, state_style)),
                    Cell::from(Span::styled(age, state_style)),
                ])
            } else {
                let context = s
                    .context_percent()
                    .map(|p| format!("{p:.0}%"))
                    .unwrap_or_else(|| "-".into());
                let rate = s
                    .tokens_per_minute
                    .map(|r| fmt_tokens(r as u64))
                    .unwrap_or_else(|| "-".into());
                let project = s.project.clone().unwrap_or_else(|| "-".into());
                let state_label = match state {
                    SessionState::Active => "active",
                    SessionState::Recent => "recent",
                    SessionState::Idle => "idle",
                };
                Row::new(vec![
                    Cell::from(Span::styled(s.provider.display_name(), provider_style)),
                    Cell::from(Span::styled(model, state_style)),
                    Cell::from(Span::styled(context, state_style)),
                    Cell::from(Span::styled(rate, state_style)),
                    Cell::from(Span::styled(tokens, state_style)),
                    Cell::from(Span::styled(project, state_style)),
                    Cell::from(Span::styled(age, state_style)),
                    Cell::from(Span::styled(state_label, state_style)),
                ])
            }
        })
        .collect();

    let widths: Vec<Constraint> = if narrow {
        vec![
            Constraint::Length(6),
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Length(5),
        ]
    } else {
        vec![
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(10),
            Constraint::Length(5),
            Constraint::Length(6),
        ]
    };

    let table = Table::new(rows, widths).header(header).block(block);
    frame.render_widget(table, area);
    let _ = inner;
}

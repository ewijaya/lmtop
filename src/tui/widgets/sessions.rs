use crate::app::App;
use crate::domain::{SessionState, SessionUsage};
use crate::tui::theme::{Theme, fmt_age, fmt_tokens};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Row, Table};

#[allow(clippy::too_many_arguments)]
pub fn render_sessions(
    frame: &mut Frame,
    area: Rect,
    sessions: &[&SessionUsage],
    app: &App,
    theme: &Theme,
    now: DateTime<Utc>,
    focused: bool,
    narrow: bool,
) {
    let border_style = if focused {
        theme.border_focused()
    } else {
        theme.border()
    };
    let mut title = format!(" SESSIONS ({}) ", sessions.len());
    if app.sort != crate::app::SortKey::Age || app.sort_reversed {
        title.push_str(&format!(
            "· sort {}{} ",
            app.sort.label(),
            if app.sort_reversed { "↑" } else { "" }
        ));
    }
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(border_style)
        .title(Span::styled(title, theme.title()));
    if app.filter_editing || !app.filter.is_empty() {
        let cursor = if app.filter_editing { "▏" } else { "" };
        block = block.title_bottom(Line::from(vec![
            Span::styled(" /", theme.title()),
            Span::styled(
                format!("{}{cursor} ", app.filter),
                Style::default()
                    .fg(theme.good())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    } else if focused {
        block = block.title_bottom(
            Line::from(Span::styled(
                " Enter detail  o sort  O reverse  / filter ",
                theme.dim(),
            ))
            .right_aligned(),
        );
    }
    let inner = block.inner(area);

    let visible_rows = inner.height.saturating_sub(1) as usize; // minus header
    let cursor = app.session_cursor.min(sessions.len().saturating_sub(1));
    // Keep the cursor row in view: scroll follows the cursor.
    let scroll = if visible_rows == 0 {
        0
    } else if cursor >= visible_rows {
        cursor + 1 - visible_rows
    } else {
        0
    };

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

    // The pulse: active sessions get a breathing marker, unless reduced
    // motion is on.
    let pulse_on = !app.reduced_motion && (app.tick / 2).is_multiple_of(2);

    let rows: Vec<Row> = sessions
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_rows)
        .map(|(i, s)| {
            let state = s.state(now);
            let selected = focused && i == cursor;
            let state_style = match state {
                SessionState::Active => Style::default().fg(theme.good()),
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
            let provider_label = if state == SessionState::Active {
                let dot = if theme.ascii {
                    "*"
                } else if pulse_on {
                    "●"
                } else {
                    "○"
                };
                format!("{dot}{}", s.provider.display_name())
            } else {
                format!(" {}", s.provider.display_name())
            };
            let row = if narrow {
                Row::new(vec![
                    Cell::from(Span::styled(provider_label, provider_style)),
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
                    Cell::from(Span::styled(provider_label, provider_style)),
                    Cell::from(Span::styled(model, state_style)),
                    Cell::from(Span::styled(context, state_style)),
                    Cell::from(Span::styled(rate, state_style)),
                    Cell::from(Span::styled(tokens, state_style)),
                    Cell::from(Span::styled(project, state_style)),
                    Cell::from(Span::styled(age, state_style)),
                    Cell::from(Span::styled(state_label, state_style)),
                ])
            };
            if selected {
                row.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                row
            }
        })
        .collect();

    let widths: Vec<Constraint> = if narrow {
        vec![
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Length(5),
        ]
    } else {
        vec![
            Constraint::Length(9),
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
}

/// Map a click at terminal row `y` (absolute) inside the sessions area to a
/// session index, honoring the cursor-follow scroll. `None` for the border,
/// header, or empty space.
pub fn session_index_at(area: Rect, y: u16, sessions_len: usize, cursor: usize) -> Option<usize> {
    // area includes the border: inner starts at area.y + 1, header row
    // occupies the first inner row.
    let first_data_row = area.y + 2;
    if y < first_data_row || y >= area.bottom().saturating_sub(1) {
        return None;
    }
    let visible_rows = (area.height.saturating_sub(3)) as usize; // borders + header
    let cursor = cursor.min(sessions_len.saturating_sub(1));
    let scroll = if visible_rows == 0 {
        0
    } else if cursor >= visible_rows {
        cursor + 1 - visible_rows
    } else {
        0
    };
    let index = scroll + (y - first_data_row) as usize;
    (index < sessions_len).then_some(index)
}

/// True when the click row is the header row of the sessions table.
pub fn is_header_row(area: Rect, y: u16) -> bool {
    y == area.y + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_maps_to_session_rows() {
        let area = Rect::new(0, 10, 100, 10); // inner rows 11..19, data 12..
        assert_eq!(session_index_at(area, 11, 5, 0), None); // header
        assert_eq!(session_index_at(area, 12, 5, 0), Some(0));
        assert_eq!(session_index_at(area, 14, 5, 0), Some(2));
        assert_eq!(session_index_at(area, 19, 5, 0), None); // border
        assert_eq!(session_index_at(area, 12, 0, 0), None); // no sessions
        assert!(is_header_row(area, 11));
        assert!(!is_header_row(area, 12));
    }

    #[test]
    fn click_accounts_for_scroll() {
        let area = Rect::new(0, 0, 100, 6); // 3 visible data rows
        // Cursor at 9 of 20 sessions → scroll = 9 + 1 - 3 = 7.
        assert_eq!(session_index_at(area, 2, 20, 9), Some(7));
        assert_eq!(session_index_at(area, 4, 20, 9), Some(9));
    }
}

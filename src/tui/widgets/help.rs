use crate::tui::layout::centered;
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

const KEYS: &[(&str, &str)] = &[
    ("1", "Codex view"),
    ("2", "Claude view"),
    ("3", "Combined view"),
    ("Tab", "Change focused panel"),
    ("s", "Focus sessions"),
    ("m", "Focus model breakdown"),
    ("w", "Focus weekly usage"),
    ("h", "Focus history chart"),
    ("j/k, ↓/↑", "Scroll sessions"),
    ("r", "Refresh now"),
    ("p", "Pause / resume"),
    ("?", "Toggle this help"),
    ("q, Esc", "Quit"),
];

pub fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let height = (KEYS.len() + 4) as u16;
    let rect = centered(area, 44, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.border_focused())
        .title(Span::styled(" HELP ", theme.title()));
    let mut lines: Vec<Line> = vec![Line::default()];
    for (key, action) in KEYS {
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:>9}  "), theme.title()),
            Span::styled(*action, theme.text()),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

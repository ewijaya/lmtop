//! Terminal UI: event loop, key/mouse handling, and render dispatch.
//! Rendering reads only [`App`] state; collector updates arrive over a
//! channel and never block drawing.

pub mod layout;
pub mod theme;
pub mod widgets;

use crate::alerts::AlertEngine;
use crate::app::{App, FilterKey, KeyAction, Panel, View};
use crate::domain::Provider;
use chrono::Utc;
use color_eyre::eyre::Result;
use ratatui::Frame;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use theme::Theme;

/// Shared control handles between the UI thread and collector tasks.
#[derive(Clone)]
pub struct CollectorControl {
    pub paused: Arc<AtomicBool>,
    pub refresh_now: Arc<tokio::sync::Notify>,
}

impl CollectorControl {
    pub fn new() -> Self {
        CollectorControl {
            paused: Arc::new(AtomicBool::new(false)),
            refresh_now: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

impl Default for CollectorControl {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run(
    mut app: App,
    mut theme: Theme,
    mut updates: tokio::sync::mpsc::Receiver<crate::domain::ProviderSnapshot>,
    control: CollectorControl,
    mut alert_engine: AlertEngine,
) -> Result<()> {
    // ratatui::init installs a panic hook that restores the terminal before
    // the panic message prints; restore() below covers normal and error exit.
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = event_loop(
        &mut terminal,
        &mut app,
        &mut theme,
        &mut updates,
        &control,
        &mut alert_engine,
    );
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    theme: &mut Theme,
    updates: &mut tokio::sync::mpsc::Receiver<crate::domain::ProviderSnapshot>,
    control: &CollectorControl,
    alert_engine: &mut AlertEngine,
) -> Result<()> {
    let tick = if app.reduced_motion {
        Duration::from_millis(1000)
    } else {
        Duration::from_millis(250)
    };
    loop {
        let now = Utc::now();
        while let Ok(snapshot) = updates.try_recv() {
            let mut ring_bell = false;
            for alert in alert_engine.check(&snapshot, now) {
                alert_engine.deliver(&alert);
                app.push_alert(alert);
                ring_bell = true;
            }
            if ring_bell && alert_engine.bell_enabled() {
                let mut out = std::io::stdout();
                let _ = out.write_all(b"\x07");
                let _ = out.flush();
            }
            app.apply_update(snapshot, now);
        }
        app.tick = app.tick.wrapping_add(1);
        terminal.draw(|frame| draw(frame, app, theme))?;

        // Wait up to one tick for input; resize events also land here.
        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.filter_editing {
                        handle_filter_event(app, key.code, key.modifiers);
                    } else if let Some(action) = map_key(key.code, key.modifiers) {
                        match action {
                            KeyAction::TogglePause => {
                                app.handle_key(action);
                                control.paused.store(app.paused, Ordering::Relaxed);
                            }
                            _ => app.handle_key(action),
                        }
                    } else if key.code == KeyCode::Char('r') {
                        control.refresh_now.notify_waiters();
                    } else if key.code == KeyCode::Char('t') {
                        theme.cycle(1);
                    } else if key.code == KeyCode::Char('T') {
                        theme.cycle(-1);
                    }
                }
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    handle_mouse(app, mouse, Rect::new(0, 0, size.width, size.height));
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        if app.should_quit {
            return Ok(());
        }
    }
}

fn handle_filter_event(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        app.handle_filter_key(FilterKey::Cancel);
        return;
    }
    match code {
        KeyCode::Esc => app.handle_filter_key(FilterKey::Cancel),
        KeyCode::Enter => app.handle_filter_key(FilterKey::Commit),
        KeyCode::Backspace => app.handle_filter_key(FilterKey::Backspace),
        KeyCode::Char(c) => app.handle_filter_key(FilterKey::Char(c)),
        _ => {}
    }
}

fn map_key(code: KeyCode, modifiers: KeyModifiers) -> Option<KeyAction> {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return Some(KeyAction::Quit);
    }
    match code {
        KeyCode::Char('q') => Some(KeyAction::Quit),
        KeyCode::Esc => Some(KeyAction::Back),
        KeyCode::Char('1') => Some(KeyAction::View(View::Codex)),
        KeyCode::Char('2') => Some(KeyAction::View(View::Claude)),
        KeyCode::Char('3') => Some(KeyAction::View(View::Combined)),
        KeyCode::Char('4') => Some(KeyAction::View(View::Planner)),
        KeyCode::Tab => Some(KeyAction::NextPanel),
        KeyCode::Char('s') => Some(KeyAction::Focus(Panel::Sessions)),
        KeyCode::Char('m') => Some(KeyAction::Focus(Panel::Breakdown)),
        KeyCode::Char('w') => Some(KeyAction::Focus(Panel::Weekly)),
        KeyCode::Char('h') => Some(KeyAction::Focus(Panel::Rate)),
        KeyCode::Char('p') => Some(KeyAction::TogglePause),
        KeyCode::Char('?') => Some(KeyAction::ToggleHelp),
        KeyCode::Down | KeyCode::Char('j') => Some(KeyAction::ScrollDown),
        KeyCode::Up | KeyCode::Char('k') => Some(KeyAction::ScrollUp),
        KeyCode::Enter => Some(KeyAction::Select),
        KeyCode::Char('v') => Some(KeyAction::ToggleChartMode),
        KeyCode::Left => Some(KeyAction::PanLeft),
        KeyCode::Right => Some(KeyAction::PanRight),
        KeyCode::Char('+') | KeyCode::Char('=') => Some(KeyAction::ZoomIn),
        KeyCode::Char('-') => Some(KeyAction::ZoomOut),
        KeyCode::Char('0') => Some(KeyAction::ResetPan),
        KeyCode::Char('o') => Some(KeyAction::CycleSort),
        KeyCode::Char('O') => Some(KeyAction::ReverseSort),
        KeyCode::Char('/') => Some(KeyAction::StartFilter),
        _ => None,
    }
}

/// Panels and their rects for the current view, for mouse hit-testing.
/// Recomputed from the pure layout functions — cheap, and avoids threading
/// mutable rect state through rendering.
fn panel_rects(app: &App, area: Rect) -> Vec<(Panel, Rect)> {
    match app.view {
        View::Combined => {
            let l = layout::combined(area, app.enabled_providers.len().max(1));
            let mut rects: Vec<(Panel, Rect)> =
                l.providers.iter().map(|r| (Panel::Providers, *r)).collect();
            rects.extend([
                (Panel::Rate, l.rate_chart),
                (Panel::Sessions, l.sessions),
                (Panel::Weekly, l.weekly),
                (Panel::Breakdown, l.breakdown),
            ]);
            rects
        }
        View::Codex | View::Claude => {
            let l = layout::provider(area);
            vec![
                (Panel::Providers, l.panel),
                (Panel::Rate, l.rate_chart),
                (Panel::Sessions, l.sessions),
                (Panel::Weekly, l.weekly),
                (Panel::Breakdown, l.breakdown),
            ]
        }
        View::Planner => Vec::new(),
    }
}

/// The sessions panel rect for the current view, if visible.
fn sessions_rect(app: &App, area: Rect) -> Option<Rect> {
    panel_rects(app, area)
        .into_iter()
        .find(|(p, _)| *p == Panel::Sessions)
        .map(|(_, r)| r)
}

fn handle_mouse(app: &mut App, mouse: MouseEvent, area: Rect) {
    let pos = Position::new(mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // An open overlay swallows clicks (click anywhere to close).
            if app.session_detail.is_some() {
                app.handle_key(KeyAction::Select);
                return;
            }
            for (panel, rect) in panel_rects(app, area) {
                if !rect.contains(pos) {
                    continue;
                }
                app.focus = panel;
                if panel == Panel::Sessions {
                    let len = app.visible_sessions().len();
                    if widgets::sessions::is_header_row(rect, mouse.row) {
                        app.handle_key(KeyAction::CycleSort);
                    } else if let Some(index) = widgets::sessions::session_index_at(
                        rect,
                        mouse.row,
                        len,
                        app.session_cursor,
                    ) {
                        if app.session_cursor == index {
                            app.handle_key(KeyAction::Select); // second click opens
                        } else {
                            app.session_cursor = index;
                        }
                    }
                }
                return;
            }
        }
        MouseEventKind::ScrollDown => {
            if sessions_rect(app, area).is_some_and(|r| r.contains(pos)) {
                app.focus = Panel::Sessions;
                app.handle_key(KeyAction::ScrollDown);
            } else if app.focus == Panel::Rate {
                app.handle_key(KeyAction::PanRight);
            }
        }
        MouseEventKind::ScrollUp => {
            if sessions_rect(app, area).is_some_and(|r| r.contains(pos)) {
                app.focus = Panel::Sessions;
                app.handle_key(KeyAction::ScrollUp);
            } else if app.focus == Panel::Rate {
                app.handle_key(KeyAction::PanLeft);
            }
        }
        _ => {}
    }
}

fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    if layout::too_small(area) {
        frame.render_widget(
            Paragraph::new(Span::styled("terminal too small", theme.dim())),
            area,
        );
        return;
    }
    let now = Utc::now();
    match app.view {
        View::Combined => draw_combined(frame, app, theme, now),
        View::Codex => draw_provider(frame, app, theme, now, Provider::Codex),
        View::Claude => draw_provider(frame, app, theme, now, Provider::Claude),
        View::Planner => draw_planner(frame, app, theme, now),
    }
    if let Some(session) = app.detail_session() {
        widgets::render_session_detail(frame, area, session, theme, now);
    }
    if app.show_help {
        widgets::render_help(frame, area, theme);
    }
}

fn draw_combined(frame: &mut Frame, app: &App, theme: &Theme, now: chrono::DateTime<Utc>) {
    let l = layout::combined(frame.area(), app.enabled_providers.len().max(1));
    widgets::render_header(frame, l.header, app, theme, now);
    for (i, provider) in app.enabled_providers.iter().enumerate() {
        let Some(rect) = l.providers.get(i) else {
            break;
        };
        widgets::render_provider_panel(
            frame,
            *rect,
            *provider,
            app.provider(*provider),
            app.history.as_ref(),
            theme,
            now,
            app.focus == Panel::Providers && i == 0,
            false,
        );
    }
    let providers: Vec<(Provider, Option<&crate::domain::ProviderSnapshot>)> = app
        .enabled_providers
        .iter()
        .map(|p| (*p, app.provider(*p)))
        .collect();
    widgets::render_chart(
        frame,
        l.rate_chart,
        &providers,
        app,
        theme,
        now,
        app.focus == Panel::Rate,
    );
    let sessions = app.visible_sessions();
    widgets::render_sessions(
        frame,
        l.sessions,
        &sessions,
        app,
        theme,
        now,
        app.focus == Panel::Sessions,
        l.narrow,
    );
    widgets::render_weekly(
        frame,
        l.weekly,
        &providers,
        theme,
        app.focus == Panel::Weekly,
    );
    widgets::render_breakdown(
        frame,
        l.breakdown,
        &providers,
        theme,
        app.focus == Panel::Breakdown,
    );
    widgets::render_footer(frame, l.footer, app, theme, now);
}

fn draw_provider(
    frame: &mut Frame,
    app: &App,
    theme: &Theme,
    now: chrono::DateTime<Utc>,
    provider: Provider,
) {
    let l = layout::provider(frame.area());
    widgets::render_header(frame, l.header, app, theme, now);
    widgets::render_provider_panel(
        frame,
        l.panel,
        provider,
        app.provider(provider),
        app.history.as_ref(),
        theme,
        now,
        app.focus == Panel::Providers,
        true,
    );
    let providers: Vec<(Provider, Option<&crate::domain::ProviderSnapshot>)> =
        vec![(provider, app.provider(provider))];
    widgets::render_chart(
        frame,
        l.rate_chart,
        &providers,
        app,
        theme,
        now,
        app.focus == Panel::Rate,
    );
    let sessions = app.visible_sessions();
    widgets::render_sessions(
        frame,
        l.sessions,
        &sessions,
        app,
        theme,
        now,
        app.focus == Panel::Sessions,
        l.narrow,
    );
    widgets::render_weekly(
        frame,
        l.weekly,
        &providers,
        theme,
        app.focus == Panel::Weekly,
    );
    widgets::render_breakdown(
        frame,
        l.breakdown,
        &providers,
        theme,
        app.focus == Panel::Breakdown,
    );
    widgets::render_footer(frame, l.footer, app, theme, now);
}

fn draw_planner(frame: &mut Frame, app: &App, theme: &Theme, now: chrono::DateTime<Utc>) {
    let l = layout::planner(frame.area(), app.enabled_providers.len().max(1));
    widgets::render_header(frame, l.header, app, theme, now);
    for (i, provider) in app.enabled_providers.iter().enumerate() {
        let Some(rect) = l.providers.get(i) else {
            break;
        };
        widgets::render_planner(
            frame,
            *rect,
            *provider,
            app.provider(*provider),
            app.history.as_ref(),
            theme,
            now,
        );
    }
    widgets::render_footer(frame, l.footer, app, theme, now);
}

//! Terminal UI: event loop, key handling, and render dispatch. Rendering
//! reads only [`App`] state; collector updates arrive over a channel and
//! never block drawing.

pub mod layout;
pub mod theme;
pub mod widgets;

use crate::app::{App, KeyAction, Panel, View};
use crate::domain::Provider;
use chrono::Utc;
use color_eyre::eyre::Result;
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
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
    theme: Theme,
    mut updates: tokio::sync::mpsc::Receiver<crate::domain::ProviderSnapshot>,
    control: CollectorControl,
    reduced_motion: bool,
) -> Result<()> {
    // ratatui::init installs a panic hook that restores the terminal before
    // the panic message prints; restore() below covers normal and error exit.
    let mut terminal = ratatui::init();
    let result = event_loop(
        &mut terminal,
        &mut app,
        &theme,
        &mut updates,
        &control,
        reduced_motion,
    );
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    theme: &Theme,
    updates: &mut tokio::sync::mpsc::Receiver<crate::domain::ProviderSnapshot>,
    control: &CollectorControl,
    reduced_motion: bool,
) -> Result<()> {
    let tick = if reduced_motion {
        Duration::from_millis(1000)
    } else {
        Duration::from_millis(250)
    };
    loop {
        let mut dirty = false;
        while let Ok(snapshot) = updates.try_recv() {
            app.apply_update(snapshot, Utc::now());
            dirty = true;
        }
        terminal.draw(|frame| draw(frame, app, theme))?;
        let _ = dirty;

        // Wait up to one tick for input; resize events also land here.
        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Some(action) = map_key(key.code, key.modifiers) {
                        match action {
                            KeyAction::TogglePause => {
                                app.handle_key(action);
                                control.paused.store(app.paused, Ordering::Relaxed);
                            }
                            _ => app.handle_key(action),
                        }
                    } else if key.code == KeyCode::Char('r') {
                        control.refresh_now.notify_waiters();
                    }
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

fn map_key(code: KeyCode, modifiers: KeyModifiers) -> Option<KeyAction> {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return Some(KeyAction::Quit);
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => Some(KeyAction::Quit),
        KeyCode::Char('1') => Some(KeyAction::View(View::Codex)),
        KeyCode::Char('2') => Some(KeyAction::View(View::Claude)),
        KeyCode::Char('3') => Some(KeyAction::View(View::Combined)),
        KeyCode::Tab => Some(KeyAction::NextPanel),
        KeyCode::Char('s') => Some(KeyAction::Focus(Panel::Sessions)),
        KeyCode::Char('m') => Some(KeyAction::Focus(Panel::Breakdown)),
        KeyCode::Char('w') => Some(KeyAction::Focus(Panel::Weekly)),
        KeyCode::Char('h') => Some(KeyAction::Focus(Panel::Rate)),
        KeyCode::Char('p') => Some(KeyAction::TogglePause),
        KeyCode::Char('?') => Some(KeyAction::ToggleHelp),
        KeyCode::Down | KeyCode::Char('j') => Some(KeyAction::ScrollDown),
        KeyCode::Up | KeyCode::Char('k') => Some(KeyAction::ScrollUp),
        _ => None,
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
    }
    if app.show_help {
        widgets::render_help(frame, area, theme);
    }
}

fn draw_combined(frame: &mut Frame, app: &App, theme: &Theme, now: chrono::DateTime<Utc>) {
    let l = layout::combined(frame.area());
    widgets::render_header(frame, l.header, app, theme, now);
    widgets::render_provider_panel(
        frame,
        l.codex_panel,
        Provider::Codex,
        app.provider(Provider::Codex),
        theme,
        now,
        app.focus == Panel::Providers,
        false,
    );
    widgets::render_provider_panel(
        frame,
        l.claude_panel,
        Provider::Claude,
        app.provider(Provider::Claude),
        theme,
        now,
        false,
        false,
    );
    let providers: Vec<(Provider, Option<&crate::domain::ProviderSnapshot>)> = Provider::ALL
        .iter()
        .map(|p| (*p, app.provider(*p)))
        .collect();
    widgets::render_rate_chart(
        frame,
        l.rate_chart,
        &providers,
        theme,
        app.focus == Panel::Rate,
    );
    let sessions = app.visible_sessions();
    widgets::render_sessions(
        frame,
        l.sessions,
        &sessions,
        theme,
        now,
        app.focus == Panel::Sessions,
        app.session_scroll,
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
        theme,
        now,
        app.focus == Panel::Providers,
        true,
    );
    let providers: Vec<(Provider, Option<&crate::domain::ProviderSnapshot>)> =
        vec![(provider, app.provider(provider))];
    widgets::render_rate_chart(
        frame,
        l.rate_chart,
        &providers,
        theme,
        app.focus == Panel::Rate,
    );
    let sessions = app.visible_sessions();
    widgets::render_sessions(
        frame,
        l.sessions,
        &sessions,
        theme,
        now,
        app.focus == Panel::Sessions,
        app.session_scroll,
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

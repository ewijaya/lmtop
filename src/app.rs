//! Central application state: the latest normalized snapshot per provider
//! plus UI state. Collector updates arrive over a channel; rendering only
//! ever reads this struct.

use crate::domain::{Freshness, Provider, ProviderSnapshot, UsageSnapshot};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Combined,
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Providers,
    Rate,
    Sessions,
    Weekly,
    Breakdown,
}

impl Panel {
    pub const ORDER: [Panel; 5] = [
        Panel::Providers,
        Panel::Rate,
        Panel::Sessions,
        Panel::Weekly,
        Panel::Breakdown,
    ];

    pub fn next(self) -> Panel {
        let i = Self::ORDER.iter().position(|p| *p == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }
}

#[derive(Debug)]
pub struct App {
    pub snapshot: UsageSnapshot,
    pub view: View,
    pub focus: Panel,
    pub show_help: bool,
    pub paused: bool,
    pub session_scroll: usize,
    pub refresh_secs: u64,
    /// Seconds after which data is considered stale.
    pub stale_after_secs: i64,
    pub should_quit: bool,
}

impl App {
    pub fn new(now: DateTime<Utc>, refresh_secs: u64) -> Self {
        App {
            snapshot: UsageSnapshot::new(now),
            view: View::Combined,
            focus: Panel::Providers,
            show_help: false,
            paused: false,
            session_scroll: 0,
            refresh_secs,
            stale_after_secs: (refresh_secs as i64 * 3).max(30),
            should_quit: false,
        }
    }

    pub fn apply_update(&mut self, snapshot: ProviderSnapshot, now: DateTime<Utc>) {
        self.snapshot.generated_at = now;
        self.snapshot.providers.insert(snapshot.provider, snapshot);
    }

    pub fn provider(&self, provider: Provider) -> Option<&ProviderSnapshot> {
        self.snapshot.providers.get(&provider)
    }

    pub fn freshness(&self, provider: Provider, now: DateTime<Utc>) -> Freshness {
        match self.provider(provider) {
            Some(p) => Freshness::from_last_scan(p.health.last_scan, now, self.stale_after_secs),
            None => Freshness::Unavailable,
        }
    }

    /// Sessions to show for the current view, across providers in the
    /// combined view, newest first.
    pub fn visible_sessions(&self) -> Vec<&crate::domain::SessionUsage> {
        let providers: &[Provider] = match self.view {
            View::Combined => &Provider::ALL,
            View::Codex => &[Provider::Codex],
            View::Claude => &[Provider::Claude],
        };
        let mut sessions: Vec<_> = providers
            .iter()
            .filter_map(|p| self.provider(*p))
            .flat_map(|p| p.sessions.iter())
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        sessions
    }

    pub fn handle_key(&mut self, code: KeyAction) {
        match code {
            KeyAction::Quit => self.should_quit = true,
            KeyAction::View(v) => {
                self.view = v;
                self.session_scroll = 0;
            }
            KeyAction::NextPanel => self.focus = self.focus.next(),
            KeyAction::Focus(panel) => self.focus = panel,
            KeyAction::TogglePause => self.paused = !self.paused,
            KeyAction::ToggleHelp => self.show_help = !self.show_help,
            KeyAction::ScrollDown => {
                if self.focus == Panel::Sessions {
                    self.session_scroll = self.session_scroll.saturating_add(1);
                }
            }
            KeyAction::ScrollUp => {
                if self.focus == Panel::Sessions {
                    self.session_scroll = self.session_scroll.saturating_sub(1);
                }
            }
        }
    }
}

/// Semantic key actions, decoupled from crossterm key codes so app logic is
/// testable without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Quit,
    View(View),
    NextPanel,
    Focus(Panel),
    TogglePause,
    ToggleHelp,
    ScrollDown,
    ScrollUp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_cycles_panels() {
        let mut app = App::new(Utc::now(), 5);
        let start = app.focus;
        for _ in 0..Panel::ORDER.len() {
            app.handle_key(KeyAction::NextPanel);
        }
        assert_eq!(app.focus, start);
    }

    #[test]
    fn view_switch_resets_scroll() {
        let mut app = App::new(Utc::now(), 5);
        app.focus = Panel::Sessions;
        app.handle_key(KeyAction::ScrollDown);
        assert_eq!(app.session_scroll, 1);
        app.handle_key(KeyAction::View(View::Codex));
        assert_eq!(app.session_scroll, 0);
        assert_eq!(app.view, View::Codex);
    }
}

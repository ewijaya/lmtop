//! Central application state: the latest normalized snapshot per provider
//! plus UI state. Collector updates arrive over a channel; rendering only
//! ever reads this struct.

use crate::alerts::Alert;
use crate::domain::{Freshness, Provider, ProviderSnapshot, UsageSnapshot};
use crate::persist::HistoryStore;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Combined,
    Codex,
    Claude,
    /// Capacity planning: race bars, burn scenarios, week pacing.
    Planner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Providers,
    Rate,
    Sessions,
    Weekly,
    Breakdown,
}

/// What the chart panel plots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartMode {
    /// Observed tokens/minute.
    Rate,
    /// Provider-reported quota percentages over time (the sawtooth).
    Quota,
}

/// Session table ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Age,
    Tokens,
    Rate,
    Project,
}

impl SortKey {
    pub const ORDER: [SortKey; 4] = [
        SortKey::Age,
        SortKey::Tokens,
        SortKey::Rate,
        SortKey::Project,
    ];

    pub fn next(self) -> SortKey {
        let i = Self::ORDER.iter().position(|s| *s == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Age => "age",
            SortKey::Tokens => "tokens",
            SortKey::Rate => "tok/min",
            SortKey::Project => "project",
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub snapshot: UsageSnapshot,
    pub view: View,
    pub focus: Panel,
    pub show_help: bool,
    pub paused: bool,
    /// Cursor row into `visible_sessions()`; the renderer keeps it visible.
    pub session_cursor: usize,
    pub refresh_secs: u64,
    /// Seconds after which data is considered stale.
    pub stale_after_secs: i64,
    pub should_quit: bool,
    /// Providers enabled by configuration, in display order.
    pub enabled_providers: Vec<Provider>,
    /// Persisted rate/quota history, when persistence is on.
    pub history: Option<HistoryStore>,
    /// Recent alerts, newest last (bounded).
    pub alerts: Vec<Alert>,
    /// Chart panel state.
    pub chart_mode: ChartMode,
    /// Pan offset back from "now", in minutes. 0 = live edge.
    pub pan_minutes: i64,
    /// Visible chart window width in minutes.
    pub zoom_minutes: i64,
    /// Session id shown in the detail overlay, if open.
    pub session_detail: Option<String>,
    pub sort: SortKey,
    pub sort_reversed: bool,
    /// Active substring filter over project/model/provider.
    pub filter: String,
    /// True while the filter line is being typed into.
    pub filter_editing: bool,
    /// Render tick counter (drives the active-session pulse).
    pub tick: u64,
    /// Disables blinking/pulsing when set.
    pub reduced_motion: bool,
}

/// How long an alert stays in the header flash area.
pub const ALERT_FLASH_SECS: i64 = 30;
/// Bound on the kept alert log.
const MAX_ALERTS: usize = 100;
/// Narrowest chart window.
pub const MIN_ZOOM_MINUTES: i64 = 15;

impl App {
    pub fn new(now: DateTime<Utc>, refresh_secs: u64) -> Self {
        App {
            snapshot: UsageSnapshot::new(now),
            view: View::Combined,
            focus: Panel::Providers,
            show_help: false,
            paused: false,
            session_cursor: 0,
            refresh_secs,
            stale_after_secs: (refresh_secs as i64 * 3).max(30),
            should_quit: false,
            enabled_providers: vec![Provider::Codex, Provider::Claude],
            history: None,
            alerts: Vec::new(),
            chart_mode: ChartMode::Rate,
            pan_minutes: 0,
            zoom_minutes: 60,
            session_detail: None,
            sort: SortKey::Age,
            sort_reversed: false,
            filter: String::new(),
            filter_editing: false,
            tick: 0,
            reduced_motion: false,
        }
    }

    pub fn apply_update(&mut self, snapshot: ProviderSnapshot, now: DateTime<Utc>) {
        if let Some(history) = &mut self.history {
            history.record(&snapshot, now);
        }
        self.snapshot.generated_at = now;
        self.snapshot.providers.insert(snapshot.provider, snapshot);
    }

    pub fn push_alert(&mut self, alert: Alert) {
        self.alerts.push(alert);
        if self.alerts.len() > MAX_ALERTS {
            self.alerts.remove(0);
        }
    }

    /// The alert currently flashing in the header, if any is fresh enough.
    pub fn flash_alert(&self, now: DateTime<Utc>) -> Option<&Alert> {
        self.alerts
            .last()
            .filter(|a| now.signed_duration_since(a.at).num_seconds() <= ALERT_FLASH_SECS)
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

    /// Providers relevant to the current view, in display order.
    pub fn view_providers(&self) -> Vec<Provider> {
        match self.view {
            View::Combined | View::Planner => self.enabled_providers.clone(),
            View::Codex => vec![Provider::Codex],
            View::Claude => vec![Provider::Claude],
        }
    }

    /// Sessions to show for the current view: filtered, sorted, cursor-able.
    pub fn visible_sessions(&self) -> Vec<&crate::domain::SessionUsage> {
        let providers = self.view_providers();
        let needle = self.filter.to_lowercase();
        let mut sessions: Vec<_> = providers
            .iter()
            .filter_map(|p| self.provider(*p))
            .flat_map(|p| p.sessions.iter())
            .filter(|s| {
                if needle.is_empty() {
                    return true;
                }
                s.project
                    .as_deref()
                    .is_some_and(|p| p.to_lowercase().contains(&needle))
                    || s.model
                        .as_ref()
                        .is_some_and(|m| m.display.to_lowercase().contains(&needle))
                    || s.provider.display_name().to_lowercase().contains(&needle)
            })
            .collect();
        match self.sort {
            SortKey::Age => sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity)),
            SortKey::Tokens => sessions.sort_by_key(|s| std::cmp::Reverse(s.tokens.total())),
            SortKey::Rate => sessions.sort_by(|a, b| {
                b.tokens_per_minute
                    .unwrap_or(0.0)
                    .partial_cmp(&a.tokens_per_minute.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortKey::Project => sessions.sort_by(|a, b| {
                a.project
                    .as_deref()
                    .unwrap_or("~")
                    .cmp(b.project.as_deref().unwrap_or("~"))
            }),
        }
        if self.sort_reversed {
            sessions.reverse();
        }
        sessions
    }

    /// The session under the cursor, if any.
    pub fn cursor_session(&self) -> Option<&crate::domain::SessionUsage> {
        let sessions = self.visible_sessions();
        sessions
            .get(self.session_cursor.min(sessions.len().saturating_sub(1)))
            .copied()
    }

    /// The session shown in the detail overlay.
    pub fn detail_session(&self) -> Option<&crate::domain::SessionUsage> {
        let id = self.session_detail.as_deref()?;
        self.snapshot
            .providers
            .values()
            .flat_map(|p| p.sessions.iter())
            .find(|s| s.id == id)
    }

    fn clamp_cursor(&mut self) {
        let len = self.visible_sessions().len();
        self.session_cursor = self.session_cursor.min(len.saturating_sub(1));
    }

    fn clamp_pan_zoom(&mut self) {
        let max_minutes = self
            .history
            .as_ref()
            .map(|h| h.retention().num_minutes())
            .unwrap_or(60)
            .max(60);
        self.zoom_minutes = self.zoom_minutes.clamp(MIN_ZOOM_MINUTES, max_minutes);
        self.pan_minutes = self
            .pan_minutes
            .clamp(0, (max_minutes - self.zoom_minutes).max(0));
    }

    pub fn handle_key(&mut self, code: KeyAction) {
        match code {
            KeyAction::Back => {
                // Contextual escape: close the topmost thing, else quit.
                if self.filter_editing {
                    self.filter_editing = false;
                    self.filter.clear();
                } else if self.session_detail.is_some() {
                    self.session_detail = None;
                } else if self.show_help {
                    self.show_help = false;
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.clamp_cursor();
                } else {
                    self.should_quit = true;
                }
            }
            KeyAction::Quit => self.should_quit = true,
            KeyAction::View(v) => {
                self.view = v;
                self.session_cursor = 0;
                self.session_detail = None;
            }
            KeyAction::Focus(panel) => self.focus = panel,
            KeyAction::TogglePause => self.paused = !self.paused,
            KeyAction::ToggleHelp => self.show_help = !self.show_help,
            KeyAction::ScrollDown => {
                if self.focus == Panel::Sessions && self.session_detail.is_none() {
                    self.session_cursor = self.session_cursor.saturating_add(1);
                    self.clamp_cursor();
                }
            }
            KeyAction::ScrollUp => {
                if self.focus == Panel::Sessions && self.session_detail.is_none() {
                    self.session_cursor = self.session_cursor.saturating_sub(1);
                }
            }
            KeyAction::Select => {
                if self.session_detail.is_some() {
                    self.session_detail = None;
                } else if self.focus == Panel::Sessions {
                    self.session_detail = self.cursor_session().map(|s| s.id.clone());
                }
            }
            KeyAction::ToggleChartMode => {
                self.chart_mode = match self.chart_mode {
                    ChartMode::Rate => ChartMode::Quota,
                    ChartMode::Quota => ChartMode::Rate,
                };
            }
            KeyAction::PanLeft => {
                self.pan_minutes += (self.zoom_minutes / 4).max(1);
                self.clamp_pan_zoom();
            }
            KeyAction::PanRight => {
                self.pan_minutes -= (self.zoom_minutes / 4).max(1);
                self.clamp_pan_zoom();
            }
            KeyAction::ZoomIn => {
                self.zoom_minutes /= 2;
                self.clamp_pan_zoom();
            }
            KeyAction::ZoomOut => {
                self.zoom_minutes *= 2;
                self.clamp_pan_zoom();
            }
            KeyAction::ResetPan => {
                self.pan_minutes = 0;
                self.zoom_minutes = 60;
            }
            KeyAction::CycleSort => {
                self.sort = self.sort.next();
                self.session_cursor = 0;
            }
            KeyAction::ReverseSort => {
                self.sort_reversed = !self.sort_reversed;
                self.session_cursor = 0;
            }
            KeyAction::StartFilter => {
                self.filter_editing = true;
                self.focus = Panel::Sessions;
            }
        }
    }

    /// A keypress while the filter line is being edited.
    pub fn handle_filter_key(&mut self, key: FilterKey) {
        match key {
            FilterKey::Char(c) => {
                self.filter.push(c);
                self.session_cursor = 0;
            }
            FilterKey::Backspace => {
                self.filter.pop();
                self.session_cursor = 0;
            }
            FilterKey::Commit => self.filter_editing = false,
            FilterKey::Cancel => {
                self.filter_editing = false;
                self.filter.clear();
                self.clamp_cursor();
            }
        }
    }
}

/// Semantic key actions, decoupled from crossterm key codes so app logic is
/// testable without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Quit,
    /// Escape: close overlay / clear filter / quit, whichever applies.
    Back,
    View(View),
    Focus(Panel),
    TogglePause,
    ToggleHelp,
    ScrollDown,
    ScrollUp,
    /// Enter: open (or close) the session detail overlay.
    Select,
    ToggleChartMode,
    PanLeft,
    PanRight,
    ZoomIn,
    ZoomOut,
    ResetPan,
    CycleSort,
    ReverseSort,
    StartFilter,
}

/// Keys routed to the filter line while it is being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKey {
    Char(char),
    Backspace,
    Commit,
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CollectorHealth, CollectorStatus, SessionUsage, TokenCounts};

    fn app_with_sessions() -> App {
        let now = Utc::now();
        let mut app = App::new(now, 5);
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
        for (id, project, total, age_min) in [
            ("a", "alpha", 100u64, 1i64),
            ("b", "beta", 300, 5),
            ("c", "gamma", 200, 3),
        ] {
            snap.sessions.push(SessionUsage {
                provider: Provider::Claude,
                id: id.into(),
                model: None,
                project: Some(project.into()),
                started_at: None,
                last_activity: Some(now - chrono::Duration::minutes(age_min)),
                tokens: TokenCounts {
                    input: total,
                    ..Default::default()
                },
                tokens_by_model: Default::default(),
                context_tokens: None,
                context_window: None,
                tokens_per_minute: None,
            });
        }
        app.apply_update(snap, now);
        app
    }

    #[test]
    fn view_switch_resets_cursor() {
        let mut app = app_with_sessions();
        app.focus = Panel::Sessions;
        app.handle_key(KeyAction::ScrollDown);
        assert_eq!(app.session_cursor, 1);
        app.handle_key(KeyAction::View(View::Claude));
        assert_eq!(app.session_cursor, 0);
        assert_eq!(app.view, View::Claude);
    }

    #[test]
    fn sort_orders_sessions() {
        let mut app = app_with_sessions();
        // Default: newest first.
        assert_eq!(app.visible_sessions()[0].id, "a");
        app.sort = SortKey::Tokens;
        assert_eq!(app.visible_sessions()[0].id, "b");
        app.sort = SortKey::Project;
        assert_eq!(app.visible_sessions()[0].project.as_deref(), Some("alpha"));
        app.sort_reversed = true;
        assert_eq!(app.visible_sessions()[0].project.as_deref(), Some("gamma"));
    }

    #[test]
    fn filter_narrows_sessions() {
        let mut app = app_with_sessions();
        app.filter = "bet".into();
        let visible = app.visible_sessions();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].project.as_deref(), Some("beta"));
    }

    #[test]
    fn filter_editing_routes_keys() {
        let mut app = app_with_sessions();
        app.handle_key(KeyAction::StartFilter);
        assert!(app.filter_editing);
        app.handle_filter_key(FilterKey::Char('g'));
        app.handle_filter_key(FilterKey::Char('a'));
        app.handle_filter_key(FilterKey::Backspace);
        assert_eq!(app.filter, "g");
        app.handle_filter_key(FilterKey::Commit);
        assert!(!app.filter_editing);
        assert_eq!(app.visible_sessions().len(), 1); // gamma
        app.handle_key(KeyAction::Back); // clears committed filter
        assert_eq!(app.visible_sessions().len(), 3);
        assert!(!app.should_quit);
    }

    #[test]
    fn enter_opens_and_closes_detail() {
        let mut app = app_with_sessions();
        app.focus = Panel::Sessions;
        app.handle_key(KeyAction::Select);
        assert_eq!(app.session_detail.as_deref(), Some("a"));
        assert!(app.detail_session().is_some());
        app.handle_key(KeyAction::Back);
        assert!(app.session_detail.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn back_quits_only_at_top_level() {
        let mut app = app_with_sessions();
        app.show_help = true;
        app.handle_key(KeyAction::Back);
        assert!(!app.should_quit);
        app.handle_key(KeyAction::Back);
        assert!(app.should_quit);
    }

    #[test]
    fn pan_and_zoom_clamp() {
        let mut app = App::new(Utc::now(), 5);
        app.handle_key(KeyAction::ZoomIn);
        app.handle_key(KeyAction::ZoomIn);
        assert_eq!(app.zoom_minutes, MIN_ZOOM_MINUTES);
        // Without persisted history, panning is capped to the live hour.
        for _ in 0..100 {
            app.handle_key(KeyAction::PanLeft);
        }
        assert!(app.pan_minutes + app.zoom_minutes <= 60);
        app.handle_key(KeyAction::ResetPan);
        assert_eq!(app.pan_minutes, 0);
        assert_eq!(app.zoom_minutes, 60);
    }

    #[test]
    fn cursor_clamps_to_visible() {
        let mut app = app_with_sessions();
        app.focus = Panel::Sessions;
        for _ in 0..10 {
            app.handle_key(KeyAction::ScrollDown);
        }
        assert_eq!(app.session_cursor, 2);
    }
}

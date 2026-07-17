//! Screen partitioning for the dashboard, with a narrow-terminal fallback.

use ratatui::layout::{Constraint, Layout, Rect};

/// Terminals narrower than this stack panels vertically.
pub const NARROW_WIDTH: u16 = 80;
/// Below this, panels are collapsed to the essentials.
pub const MIN_WIDTH: u16 = 30;
pub const MIN_HEIGHT: u16 = 10;

#[derive(Debug, Clone, Copy)]
pub struct CombinedLayout {
    pub header: Rect,
    pub codex_panel: Rect,
    pub claude_panel: Rect,
    pub rate_chart: Rect,
    pub sessions: Rect,
    pub weekly: Rect,
    pub breakdown: Rect,
    pub footer: Rect,
    pub narrow: bool,
}

/// True when the terminal is too small to render anything meaningful.
pub fn too_small(area: Rect) -> bool {
    area.width < MIN_WIDTH || area.height < MIN_HEIGHT
}

pub fn combined(area: Rect) -> CombinedLayout {
    let narrow = area.width < NARROW_WIDTH;
    let short = area.height < 32;
    let panel_h = if short { 7 } else { 9 };
    let chart_h = if short { 7 } else { 9 };
    let bottom_h = if short { 7 } else { 9 };

    if narrow {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(panel_h),
            Constraint::Length(panel_h),
            Constraint::Length(chart_h),
            Constraint::Min(5),
            Constraint::Length(bottom_h),
            Constraint::Length(1),
        ])
        .split(area);
        // In narrow mode weekly and breakdown share one region split in two.
        let bottom = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[5]);
        CombinedLayout {
            header: rows[0],
            codex_panel: rows[1],
            claude_panel: rows[2],
            rate_chart: rows[3],
            sessions: rows[4],
            weekly: bottom[0],
            breakdown: bottom[1],
            footer: rows[6],
            narrow,
        }
    } else {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(panel_h),
            Constraint::Length(chart_h),
            Constraint::Min(6),
            Constraint::Length(bottom_h),
            Constraint::Length(1),
        ])
        .split(area);
        let providers =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(rows[1]);
        let bottom = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[4]);
        CombinedLayout {
            header: rows[0],
            codex_panel: providers[0],
            claude_panel: providers[1],
            rate_chart: rows[2],
            sessions: rows[3],
            weekly: bottom[0],
            breakdown: bottom[1],
            footer: rows[5],
            narrow,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderLayout {
    pub header: Rect,
    pub panel: Rect,
    pub rate_chart: Rect,
    pub sessions: Rect,
    pub weekly: Rect,
    pub breakdown: Rect,
    pub footer: Rect,
    pub narrow: bool,
}

/// Layout for the single-provider views (keys 1 and 2).
pub fn provider(area: Rect) -> ProviderLayout {
    let narrow = area.width < NARROW_WIDTH;
    let short = area.height < 32;
    let panel_h = if short { 8 } else { 10 };
    let chart_h = if short { 7 } else { 10 };
    let bottom_h = if short { 7 } else { 10 };
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(panel_h),
        Constraint::Length(chart_h),
        Constraint::Min(6),
        Constraint::Length(bottom_h),
        Constraint::Length(1),
    ])
    .split(area);
    let bottom =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[4]);
    ProviderLayout {
        header: rows[0],
        panel: rows[1],
        rate_chart: rows[2],
        sessions: rows[3],
        weekly: bottom[0],
        breakdown: bottom[1],
        footer: rows[5],
        narrow,
    }
}

/// Centered rectangle used by the help overlay.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_layout_puts_providers_side_by_side() {
        let l = combined(Rect::new(0, 0, 120, 40));
        assert!(!l.narrow);
        assert_eq!(l.codex_panel.y, l.claude_panel.y);
        assert!(l.claude_panel.x > l.codex_panel.x);
    }

    #[test]
    fn narrow_layout_stacks_providers() {
        let l = combined(Rect::new(0, 0, 60, 40));
        assert!(l.narrow);
        assert_eq!(l.codex_panel.x, l.claude_panel.x);
        assert!(l.claude_panel.y > l.codex_panel.y);
    }

    #[test]
    fn layouts_fit_within_area() {
        for (w, h) in [(30u16, 12u16), (60, 24), (100, 30), (200, 60)] {
            let area = Rect::new(0, 0, w, h);
            let l = combined(area);
            for r in [
                l.header,
                l.codex_panel,
                l.claude_panel,
                l.rate_chart,
                l.sessions,
                l.weekly,
                l.breakdown,
                l.footer,
            ] {
                assert!(r.right() <= area.right() && r.bottom() <= area.bottom());
            }
        }
    }

    #[test]
    fn too_small_detection() {
        assert!(too_small(Rect::new(0, 0, 20, 5)));
        assert!(!too_small(Rect::new(0, 0, 100, 30)));
    }
}

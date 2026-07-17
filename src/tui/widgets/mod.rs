mod breakdown;
mod header;
mod help;
mod planner;
mod provider_panel;
mod rate_chart;
mod session_detail;
pub mod sessions;

pub use breakdown::{render_breakdown, render_weekly};
pub use header::{render_footer, render_header};
pub use help::render_help;
pub use planner::render_planner;
pub use provider_panel::render_provider_panel;
pub use rate_chart::render_chart;
pub use session_detail::render_session_detail;
pub use sessions::render_sessions;

/// Build a textual usage bar of `width` cells for a 0..=100 percentage.
pub fn bar(width: usize, percent: f64, ascii: bool) -> String {
    let width = width.max(1);
    let clamped = percent.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let (fill, empty) = if ascii { ('#', '.') } else { ('█', '░') };
    let mut s = String::with_capacity(width * 3);
    for _ in 0..filled {
        s.push(fill);
    }
    for _ in filled..width {
        s.push(empty);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_fills_proportionally() {
        assert_eq!(bar(10, 0.0, true), "..........");
        assert_eq!(bar(10, 50.0, true), "#####.....");
        assert_eq!(bar(10, 100.0, true), "##########");
        // Out-of-range values are clamped, never panic.
        assert_eq!(bar(10, 150.0, true), "##########");
        assert_eq!(bar(10, -5.0, true), "..........");
    }

    #[test]
    fn bar_unicode_mode() {
        assert_eq!(bar(4, 50.0, false), "██░░");
    }
}

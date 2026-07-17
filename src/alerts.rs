//! Quota alerting: watches provider-reported windows for threshold
//! crossings and imminent exhaustion, fires each alert once per window
//! occurrence, and delivers via terminal bell, desktop notification, and an
//! optional user command. Pure state machine here; the TUI owns the bell.

use crate::config::AlertsConfig;
use crate::domain::{ProviderSnapshot, QuotaOutlook};
use chrono::{DateTime, Utc};
use std::collections::HashSet;

/// Skip windows whose data is older than this: alerting on hours-old local
/// snapshots is noise, not signal.
const MAX_DATA_AGE_MINUTES: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Critical,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub at: DateTime<Utc>,
    pub severity: Severity,
    pub provider: &'static str,
    pub title: String,
    pub body: String,
}

/// Deduplicating alert state. Keys embed the window's reset time, so a
/// window rolling over re-arms every alert for it.
#[derive(Debug)]
pub struct AlertEngine {
    cfg: AlertsConfig,
    fired: HashSet<String>,
}

impl AlertEngine {
    pub fn new(cfg: AlertsConfig) -> Self {
        AlertEngine {
            cfg,
            fired: HashSet::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    pub fn bell_enabled(&self) -> bool {
        self.cfg.bell
    }

    /// Evaluate one provider snapshot; returns newly fired alerts.
    pub fn check(&mut self, snap: &ProviderSnapshot, now: DateTime<Utc>) -> Vec<Alert> {
        if !self.cfg.enabled {
            return Vec::new();
        }
        let mut alerts = Vec::new();
        for w in &snap.quota_windows {
            if w.is_expired(now) {
                continue;
            }
            let age_min = now.signed_duration_since(w.captured_at).num_minutes();
            if age_min > MAX_DATA_AGE_MINUTES {
                continue;
            }
            let window_id = format!(
                "{}|{}|{}|{}",
                snap.provider,
                w.label(),
                w.scope.clone().unwrap_or_default(),
                w.resets_at.map(|t| t.timestamp()).unwrap_or(0),
            );

            let mut thresholds: Vec<f64> = self
                .cfg
                .quota_thresholds
                .iter()
                .copied()
                .filter(|t| (1.0..=100.0).contains(t))
                .collect();
            thresholds.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            // Only the highest crossed threshold fires; lower ones are
            // marked fired silently so backfilled data doesn't spam.
            let mut already_reported = false;
            for t in thresholds {
                if w.used_percent < t {
                    continue;
                }
                let key = format!("{window_id}|t{t}");
                if !self.fired.insert(key) {
                    already_reported = true;
                    continue;
                }
                if already_reported {
                    continue;
                }
                already_reported = true;
                let severity = if t >= 95.0 {
                    Severity::Critical
                } else {
                    Severity::Warning
                };
                let reset = w
                    .resets_at
                    .map(|r| {
                        format!(
                            ", resets in {}",
                            crate::tui::theme::fmt_duration_until(
                                r.signed_duration_since(now).num_seconds()
                            )
                        )
                    })
                    .unwrap_or_default();
                alerts.push(Alert {
                    at: now,
                    severity,
                    provider: snap.provider.display_name(),
                    title: format!(
                        "{} {} at {:.0}%",
                        snap.provider.display_name(),
                        w.label(),
                        w.used_percent
                    ),
                    body: format!("crossed {t:.0}%{reset}"),
                });
            }

            // Imminent exhaustion, independent of percentage thresholds.
            if let QuotaOutlook::AtRisk {
                projected_exhaustion,
            } = w.outlook()
            {
                let minutes_left = projected_exhaustion
                    .signed_duration_since(now)
                    .num_minutes();
                if minutes_left >= 0 && minutes_left <= self.cfg.exhaustion_warn_minutes as i64 {
                    let key = format!("{window_id}|exhaustion");
                    if self.fired.insert(key) {
                        alerts.push(Alert {
                            at: now,
                            severity: Severity::Critical,
                            provider: snap.provider.display_name(),
                            title: format!(
                                "{} {} projected empty in {}",
                                snap.provider.display_name(),
                                w.label(),
                                crate::tui::theme::fmt_duration_until(minutes_left * 60),
                            ),
                            body: format!("at {:.0}% and burning", w.used_percent),
                        });
                    }
                }
            }
        }
        alerts
    }

    /// Best-effort external delivery (desktop notification + user command).
    /// Failures are silent by design: alerting must never break the TUI.
    pub fn deliver(&self, alert: &Alert) {
        use std::process::{Command, Stdio};
        if self.cfg.desktop {
            #[cfg(target_os = "macos")]
            let cmd = {
                let mut c = Command::new("osascript");
                c.arg("-e").arg(format!(
                    "display notification \"{}\" with title \"{} — {}\"",
                    alert.body.replace('"', ""),
                    crate::branding::APP_NAME,
                    alert.title.replace('"', ""),
                ));
                c
            };
            #[cfg(not(target_os = "macos"))]
            let cmd = {
                let mut c = Command::new("notify-send");
                c.arg("-u")
                    .arg(match alert.severity {
                        Severity::Critical => "critical",
                        Severity::Warning => "normal",
                    })
                    .arg(format!("{} — {}", crate::branding::APP_NAME, alert.title))
                    .arg(&alert.body);
                c
            };
            let mut cmd = cmd;
            let _ = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
        if let Some(user_cmd) = &self.cfg.command
            && !user_cmd.trim().is_empty()
        {
            let _ = Command::new("bash")
                .arg("-c")
                .arg(user_cmd)
                .env("LMTOP_ALERT_TITLE", &alert.title)
                .env("LMTOP_ALERT_BODY", &alert.body)
                .env("LMTOP_ALERT_SEVERITY", alert.severity.label())
                .env("LMTOP_ALERT_PROVIDER", alert.provider)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        CollectorHealth, CollectorStatus, Provider, QuotaWindow, QuotaWindowKind,
    };

    fn snapshot(windows: Vec<QuotaWindow>) -> ProviderSnapshot {
        let mut s = ProviderSnapshot::empty(
            Provider::Claude,
            CollectorHealth {
                status: CollectorStatus::Ok,
                message: None,
                last_scan: None,
                files_scanned: 0,
                parse_errors: 0,
            },
        );
        s.quota_windows = windows;
        s
    }

    fn window(pct: f64, now: DateTime<Utc>) -> QuotaWindow {
        QuotaWindow {
            kind: QuotaWindowKind::FiveHour,
            used_percent: pct,
            window_minutes: Some(300),
            resets_at: Some(now + chrono::Duration::hours(2)),
            captured_at: now,
            scope: None,
            burn_per_hour: None,
            projected_exhaustion: None,
            trend_confidence: None,
        }
    }

    fn engine() -> AlertEngine {
        AlertEngine::new(AlertsConfig::default())
    }

    #[test]
    fn fires_once_per_threshold_crossing() {
        let now = Utc::now();
        let mut e = engine();
        assert!(e.check(&snapshot(vec![window(50.0, now)]), now).is_empty());
        let fired = e.check(&snapshot(vec![window(82.0, now)]), now);
        assert_eq!(fired.len(), 1);
        assert!(fired[0].body.contains("80"));
        // Same window, same threshold: silent.
        assert!(e.check(&snapshot(vec![window(83.0, now)]), now).is_empty());
        // Next threshold fires.
        let fired = e.check(&snapshot(vec![window(96.0, now)]), now);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].severity, Severity::Critical);
    }

    #[test]
    fn jumping_straight_to_critical_fires_only_highest() {
        let now = Utc::now();
        let mut e = engine();
        let fired = e.check(&snapshot(vec![window(97.0, now)]), now);
        assert_eq!(fired.len(), 1);
        assert!(fired[0].body.contains("95"));
    }

    #[test]
    fn new_window_rearms() {
        let now = Utc::now();
        let mut e = engine();
        let mut w = window(85.0, now);
        assert_eq!(e.check(&snapshot(vec![w.clone()]), now).len(), 1);
        // Same percentage but a later reset time: a fresh window.
        w.resets_at = Some(now + chrono::Duration::hours(7));
        assert_eq!(e.check(&snapshot(vec![w]), now).len(), 1);
    }

    #[test]
    fn stale_data_never_alerts() {
        let now = Utc::now();
        let mut e = engine();
        let mut w = window(99.0, now);
        w.captured_at = now - chrono::Duration::hours(3);
        assert!(e.check(&snapshot(vec![w]), now).is_empty());
    }

    #[test]
    fn imminent_exhaustion_fires() {
        let now = Utc::now();
        let mut e = engine();
        let mut w = window(70.0, now);
        w.burn_per_hour = Some(40.0);
        w.projected_exhaustion = Some(now + chrono::Duration::minutes(20));
        let fired = e.check(&snapshot(vec![w]), now);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].severity, Severity::Critical);
        assert!(fired[0].title.contains("projected empty"));
    }

    #[test]
    fn disabled_engine_is_silent() {
        let now = Utc::now();
        let cfg = AlertsConfig {
            enabled: false,
            ..Default::default()
        };
        let mut e = AlertEngine::new(cfg);
        assert!(e.check(&snapshot(vec![window(99.0, now)]), now).is_empty());
    }
}

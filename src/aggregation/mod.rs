//! Provider-independent aggregation: calendar-week windows, per-minute
//! history bucketing, and snapshot assembly from normalized usage events.

use crate::domain::{HistorySample, ModelIdentity, ModelWeekUsage, TokenCounts, WeekAggregate};
use chrono::{DateTime, Datelike, Duration, FixedOffset, Offset, TimeZone, Utc};
use std::collections::BTreeMap;

/// One deduplicated, delta-form usage observation: "this session consumed
/// these tokens at this time with this model". Collectors are responsible
/// for converting cumulative counters to deltas and for deduplication;
/// aggregation trusts events to be unique.
#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub model: Option<ModelIdentity>,
    pub tokens: TokenCounts,
}

/// Inclusive start / exclusive end of the calendar week containing `now`,
/// computed in the user's display timezone.
pub fn week_bounds(
    now: DateTime<Utc>,
    week_start_day: chrono::Weekday,
    fixed_offset_hours: Option<i32>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let offset = display_offset(now, fixed_offset_hours);
    let local = now.with_timezone(&offset);
    let days_back = (local.weekday().num_days_from_monday() as i64
        - week_start_day.num_days_from_monday() as i64)
        .rem_euclid(7);
    let start_date = local.date_naive() - Duration::days(days_back);
    // Midnight at the week start; on DST-ambiguous days pick the earliest
    // valid interpretation so the week always has a boundary.
    let start_local = start_date
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always valid");
    let start = offset
        .from_local_datetime(&start_local)
        .earliest()
        .expect("fixed offsets have no gaps");
    let end = start + Duration::days(7);
    (start.with_timezone(&Utc), end.with_timezone(&Utc))
}

/// The offset used for "local" calendar math: an explicit fixed offset from
/// config, or the system's current UTC offset.
fn display_offset(now: DateTime<Utc>, fixed_offset_hours: Option<i32>) -> FixedOffset {
    match fixed_offset_hours {
        Some(h) => FixedOffset::east_opt(h * 3600).unwrap_or_else(|| Utc.fix()),
        None => {
            let local = now.with_timezone(&chrono::Local);
            *local.offset()
        }
    }
}

/// Aggregate events falling inside `[week_start, week_end)` into a
/// calendar-week summary with a per-model breakdown.
pub fn build_week(
    events: &[UsageEvent],
    week_start: DateTime<Utc>,
    week_end: DateTime<Utc>,
) -> WeekAggregate {
    let mut tokens = TokenCounts::default();
    let mut by_model: BTreeMap<String, ModelWeekUsage> = BTreeMap::new();
    let mut sessions = std::collections::BTreeSet::new();
    for ev in events {
        if ev.timestamp < week_start || ev.timestamp >= week_end {
            continue;
        }
        tokens.add(&ev.tokens);
        sessions.insert(ev.session_id.as_str());
        let model = ev
            .model
            .clone()
            .unwrap_or_else(|| ModelIdentity::normalize("unknown"));
        by_model
            .entry(model.raw.clone())
            .or_insert_with(|| ModelWeekUsage {
                model,
                tokens: TokenCounts::default(),
            })
            .tokens
            .add(&ev.tokens);
    }
    WeekAggregate {
        week_start,
        week_end,
        tokens,
        by_model,
        sessions: sessions.len() as u64,
    }
}

/// Bucket recent events into per-minute throughput samples covering
/// `[now - retention, now]`. Empty minutes get explicit zero samples so
/// charts show gaps honestly.
pub fn build_history(
    events: &[UsageEvent],
    now: DateTime<Utc>,
    retention_minutes: u64,
) -> Vec<HistorySample> {
    let retention = retention_minutes as i64;
    let end_minute = truncate_to_minute(now);
    let start_minute = end_minute - Duration::minutes(retention - 1);
    let mut buckets: BTreeMap<DateTime<Utc>, (u64, u64)> = BTreeMap::new();
    let mut cursor = start_minute;
    while cursor <= end_minute {
        buckets.insert(cursor, (0, 0));
        cursor += Duration::minutes(1);
    }
    for ev in events {
        let minute = truncate_to_minute(ev.timestamp);
        if let Some((input, output)) = buckets.get_mut(&minute) {
            *input += ev.tokens.total_input();
            *output += ev.tokens.output;
        }
    }
    buckets
        .into_iter()
        .map(|(at, (input_tokens, output_tokens))| HistorySample {
            at,
            input_tokens,
            output_tokens,
        })
        .collect()
}

fn truncate_to_minute(t: DateTime<Utc>) -> DateTime<Utc> {
    let secs = t.timestamp() - t.timestamp().rem_euclid(60);
    Utc.timestamp_opt(secs, 0).single().unwrap_or(t)
}

/// Observed tokens per minute for one session over its recent activity,
/// using a short lookback window.
pub fn tokens_per_minute(
    events: &[UsageEvent],
    session_id: &str,
    now: DateTime<Utc>,
    lookback_minutes: i64,
) -> Option<f64> {
    let cutoff = now - Duration::minutes(lookback_minutes);
    let total: u64 = events
        .iter()
        .filter(|e| e.session_id == session_id && e.timestamp >= cutoff)
        .map(|e| e.tokens.total())
        .sum();
    if total == 0 {
        None
    } else {
        Some(total as f64 / lookback_minutes as f64)
    }
}

/// One provider-reported percentage observation for a quota window,
/// used to estimate burn velocity.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaSample {
    pub captured_at: DateTime<Utc>,
    pub used_percent: f64,
}

/// Samples older than this cannot anchor a forward projection: burn rates
/// from a session that ended hours ago say nothing about what happens next.
const QUOTA_TREND_MAX_AGE_MINUTES: i64 = 30;

/// Estimate burn velocity (percentage points per hour) and projected
/// exhaustion for a quota window from a series of provider-reported
/// percentage samples.
///
/// Only the most recent monotonically non-decreasing run of samples is
/// used: a drop in `used_percent` means the window rolled over or reset,
/// and mixing pre-reset samples into the trend would corrupt the estimate.
/// Stale trends (latest sample older than ~30 minutes) yield no estimate
/// at all — extrapolating an old burn rate would fabricate a forecast.
/// Returns `(burn_per_hour, projected_exhaustion)`.
pub fn project_quota(
    samples: &[QuotaSample],
    now: DateTime<Utc>,
) -> (Option<f64>, Option<DateTime<Utc>>) {
    let mut sorted: Vec<&QuotaSample> = samples.iter().collect();
    sorted.sort_by_key(|s| s.captured_at);
    let Some(last) = sorted.last() else {
        return (None, None);
    };
    if now.signed_duration_since(last.captured_at) > Duration::minutes(QUOTA_TREND_MAX_AGE_MINUTES)
    {
        return (None, None);
    }

    // Walk backwards while the percentage is non-increasing (going back in
    // time), i.e. the run is non-decreasing forward in time.
    let mut start = sorted.len() - 1;
    while start > 0 && sorted[start - 1].used_percent <= sorted[start].used_percent {
        start -= 1;
    }
    let run = &sorted[start..];
    let first = run[0];
    let span_hours = last
        .captured_at
        .signed_duration_since(first.captured_at)
        .num_seconds() as f64
        / 3600.0;
    // Need a meaningful baseline: at least two samples a few minutes apart.
    if run.len() < 2 || span_hours < 0.05 {
        return (None, None);
    }
    let burn = (last.used_percent - first.used_percent) / span_hours;
    if burn <= f64::EPSILON {
        return (Some(0.0), None);
    }
    let remaining = (100.0 - last.used_percent).max(0.0);
    let hours_left = remaining / burn;
    // Project from "now", not the sample time, so a slightly old sample
    // never projects exhaustion into the past.
    let base = last.captured_at.max(now);
    let exhaustion = base + Duration::seconds((hours_left * 3600.0) as i64);
    (Some(burn), Some(exhaustion))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    fn ev(session: &str, ts: &str, model: Option<&str>, input: u64, output: u64) -> UsageEvent {
        UsageEvent {
            session_id: session.into(),
            timestamp: ts.parse().unwrap(),
            model: model.map(ModelIdentity::normalize),
            tokens: TokenCounts {
                input,
                output,
                ..Default::default()
            },
        }
    }

    #[test]
    fn week_bounds_monday_start_fixed_offset() {
        // 2026-07-17 is a Friday. With UTC+9 and Monday start, the week
        // begins Monday 2026-07-13 00:00 +09:00 = Sunday 2026-07-12 15:00 UTC.
        let now: DateTime<Utc> = "2026-07-17T05:00:00Z".parse().unwrap();
        let (start, end) = week_bounds(now, Weekday::Mon, Some(9));
        assert_eq!(
            start,
            "2026-07-12T15:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(
            end,
            "2026-07-19T15:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn week_bounds_sunday_start() {
        let now: DateTime<Utc> = "2026-07-17T05:00:00Z".parse().unwrap();
        let (start, _) = week_bounds(now, Weekday::Sun, Some(0));
        // Sunday 2026-07-12 at 00:00 UTC.
        assert_eq!(
            start,
            "2026-07-12T00:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn week_bounds_on_the_boundary_day() {
        // Exactly at the week start instant, the week starts now.
        let now: DateTime<Utc> = "2026-07-13T00:00:00Z".parse().unwrap();
        let (start, _) = week_bounds(now, Weekday::Mon, Some(0));
        assert_eq!(start, now);
    }

    #[test]
    fn week_bounds_negative_offset() {
        // Friday 2026-07-17 01:00 UTC is still Thursday in UTC-5,
        // so the Monday-start week begins Monday 2026-07-13 00:00 -05:00.
        let now: DateTime<Utc> = "2026-07-17T01:00:00Z".parse().unwrap();
        let (start, _) = week_bounds(now, Weekday::Mon, Some(-5));
        assert_eq!(
            start,
            "2026-07-13T05:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn build_week_filters_and_groups() {
        let start: DateTime<Utc> = "2026-07-13T00:00:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-07-20T00:00:00Z".parse().unwrap();
        let events = vec![
            ev(
                "s1",
                "2026-07-14T10:00:00Z",
                Some("claude-fable-5"),
                100,
                50,
            ),
            ev("s1", "2026-07-14T10:05:00Z", Some("claude-opus-4-8"), 10, 5),
            ev(
                "s2",
                "2026-07-15T10:00:00Z",
                Some("claude-fable-5"),
                200,
                100,
            ),
            // Outside the week: excluded.
            ev(
                "s3",
                "2026-07-12T23:59:59Z",
                Some("claude-fable-5"),
                999,
                999,
            ),
            ev(
                "s3",
                "2026-07-20T00:00:00Z",
                Some("claude-fable-5"),
                999,
                999,
            ),
        ];
        let week = build_week(&events, start, end);
        assert_eq!(week.tokens.input, 310);
        assert_eq!(week.tokens.output, 155);
        assert_eq!(week.sessions, 2);
        assert_eq!(week.by_model.len(), 2);
        assert_eq!(week.by_model["claude-fable-5"].tokens.input, 300);
        assert_eq!(week.by_model["claude-opus-4-8"].tokens.output, 5);
    }

    #[test]
    fn history_buckets_by_minute_with_zero_fill() {
        let now: DateTime<Utc> = "2026-07-17T10:05:30Z".parse().unwrap();
        let events = vec![
            ev("s1", "2026-07-17T10:04:10Z", None, 60, 30),
            ev("s1", "2026-07-17T10:04:50Z", None, 40, 20),
            // Too old for a 5-minute retention window.
            ev("s1", "2026-07-17T09:00:00Z", None, 999, 999),
        ];
        let history = build_history(&events, now, 5);
        assert_eq!(history.len(), 5);
        let bucket = history
            .iter()
            .find(|s| s.at == "2026-07-17T10:04:00Z".parse::<DateTime<Utc>>().unwrap())
            .unwrap();
        assert_eq!(bucket.input_tokens, 100);
        assert_eq!(bucket.output_tokens, 50);
        assert!(history.iter().filter(|s| s.input_tokens == 0).count() >= 4);
    }

    fn sample(ts: &str, pct: f64) -> QuotaSample {
        QuotaSample {
            captured_at: ts.parse().unwrap(),
            used_percent: pct,
        }
    }

    #[test]
    fn quota_projection_from_increasing_samples() {
        // 10%/hour burn from 50%: 5 hours to exhaustion.
        let now: DateTime<Utc> = "2026-07-17T12:00:00Z".parse().unwrap();
        let samples = vec![
            sample("2026-07-17T10:00:00Z", 30.0),
            sample("2026-07-17T11:00:00Z", 40.0),
            sample("2026-07-17T12:00:00Z", 50.0),
        ];
        let (burn, exhaustion) = project_quota(&samples, now);
        assert!((burn.unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(
            exhaustion.unwrap(),
            "2026-07-17T17:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn quota_projection_ignores_pre_reset_samples() {
        // A drop (90 -> 5) means the window reset; only the trailing run
        // (5 -> 15 over 1h = 10%/h) may inform the trend.
        let now: DateTime<Utc> = "2026-07-17T12:00:00Z".parse().unwrap();
        let samples = vec![
            sample("2026-07-17T09:00:00Z", 90.0),
            sample("2026-07-17T11:00:00Z", 5.0),
            sample("2026-07-17T12:00:00Z", 15.0),
        ];
        let (burn, _) = project_quota(&samples, now);
        assert!((burn.unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn quota_projection_needs_two_samples() {
        let now: DateTime<Utc> = "2026-07-17T12:00:00Z".parse().unwrap();
        assert_eq!(project_quota(&[], now), (None, None));
        let one = vec![sample("2026-07-17T12:00:00Z", 40.0)];
        assert_eq!(project_quota(&one, now), (None, None));
    }

    #[test]
    fn quota_projection_flat_usage_has_no_exhaustion() {
        let now: DateTime<Utc> = "2026-07-17T12:00:00Z".parse().unwrap();
        let samples = vec![
            sample("2026-07-17T10:00:00Z", 40.0),
            sample("2026-07-17T12:00:00Z", 40.0),
        ];
        let (burn, exhaustion) = project_quota(&samples, now);
        assert_eq!(burn, Some(0.0));
        assert_eq!(exhaustion, None);
    }

    #[test]
    fn quota_projection_refuses_stale_trends() {
        // Last capture hours before "now": extrapolating that burn rate
        // would fabricate a forecast, so no estimate is produced.
        let now: DateTime<Utc> = "2026-07-17T12:00:00Z".parse().unwrap();
        let samples = vec![
            sample("2026-07-17T08:00:00Z", 98.0),
            sample("2026-07-17T09:00:00Z", 99.0),
        ];
        assert_eq!(project_quota(&samples, now), (None, None));
    }

    #[test]
    fn quota_projection_from_slightly_old_sample_stays_in_future() {
        let now: DateTime<Utc> = "2026-07-17T12:10:00Z".parse().unwrap();
        let samples = vec![
            sample("2026-07-17T11:00:00Z", 98.0),
            sample("2026-07-17T12:00:00Z", 99.9),
        ];
        let (_, exhaustion) = project_quota(&samples, now);
        assert!(exhaustion.unwrap() >= now);
    }

    #[test]
    fn rate_uses_only_recent_events_for_session() {
        let now: DateTime<Utc> = "2026-07-17T10:10:00Z".parse().unwrap();
        let events = vec![
            ev("s1", "2026-07-17T10:08:00Z", None, 500, 500),
            ev("s2", "2026-07-17T10:08:00Z", None, 999, 999),
            ev("s1", "2026-07-17T09:00:00Z", None, 999, 999),
        ];
        let rate = tokens_per_minute(&events, "s1", now, 10).unwrap();
        assert!((rate - 100.0).abs() < 1e-9);
        assert!(tokens_per_minute(&events, "s9", now, 10).is_none());
    }
}

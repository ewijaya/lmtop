//! Integration tests: whole collectors driven over synthetic fixtures with
//! a deterministic clock. No real session data is used.

use agentop::collectors::{Collector, ScanContext, claude::ClaudeCollector, codex::CodexCollector};
use agentop::domain::{
    Capability, CollectorStatus, ModelFamily, Provider, QuotaOutlook, QuotaWindowKind,
};
use chrono::{DateTime, Utc};
use std::path::PathBuf;

/// Deterministic clock: Friday 2026-07-17 12:00 UTC, week (Monday start,
/// UTC) covering 2026-07-13 .. 2026-07-20.
fn ctx() -> ScanContext {
    ScanContext {
        now: t("2026-07-17T12:00:00Z"),
        week_start: t("2026-07-13T00:00:00Z"),
        week_end: t("2026-07-20T00:00:00Z"),
        history_retention_minutes: 180,
    }
}

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

fn codex(rel: &str) -> CodexCollector {
    CodexCollector::with_dirs(vec![fixture(rel)])
}

fn claude(rel: &str) -> ClaudeCollector {
    ClaudeCollector::with_dirs(vec![fixture(rel)])
}

// ---------------------------------------------------------------- Codex --

#[test]
fn codex_parses_cumulative_counters_to_deltas() {
    let snap = codex("codex/basic/sessions").scan(&ctx());
    assert_eq!(snap.provider, Provider::Codex);
    assert_eq!(snap.health.status, CollectorStatus::Ok);

    let week = snap.week.as_ref().unwrap();
    // Session A only (the duplicate file must not double-count):
    // input excl. cached 1800, cached 1400, output 500, reasoning 60.
    assert_eq!(week.tokens.input, 1800);
    assert_eq!(week.tokens.cached_input, 1400);
    assert_eq!(week.tokens.output, 500);
    assert_eq!(week.tokens.reasoning, 60);
    assert_eq!(week.tokens.total(), 3700);
    assert_eq!(week.sessions, 1);
}

#[test]
fn codex_attributes_model_changes_within_a_session() {
    let snap = codex("codex/basic/sessions").scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    let terra = &week.by_model["gpt-5.6-terra"];
    let gpt55 = &week.by_model["gpt-5.5"];
    assert_eq!(terra.tokens.input, 1300);
    assert_eq!(terra.tokens.cached_input, 900);
    assert_eq!(terra.tokens.output, 350);
    assert_eq!(gpt55.tokens.input, 500);
    assert_eq!(gpt55.tokens.output, 150);
    assert_eq!(terra.model.family, ModelFamily::Gpt);
}

#[test]
fn codex_deduplicates_archived_copy_of_live_session() {
    let snap = codex("codex/basic/sessions").scan(&ctx());
    // Both files claim session codex-aaaa; only one may contribute.
    assert_eq!(snap.sessions.len(), 1);
    assert_eq!(snap.sessions[0].id, "codex-aaaa");
    assert_eq!(snap.sessions[0].tokens.total(), 3700);
    assert_eq!(snap.sessions[0].project.as_deref(), Some("alpha"));
}

#[test]
fn codex_classifies_quota_windows_by_duration_not_position() {
    let snap = codex("codex/basic/sessions").scan(&ctx());
    // Fixture deliberately reports the WEEKLY window as `primary` and the
    // five-hour window as `secondary`.
    let weekly = snap.quota_window(&QuotaWindowKind::Weekly).unwrap();
    let five = snap.quota_window(&QuotaWindowKind::FiveHour).unwrap();
    assert!((weekly.used_percent - 42.0).abs() < 1e-9);
    assert!((five.used_percent - 30.0).abs() < 1e-9);
    assert_eq!(weekly.window_minutes, Some(10_080));
    assert_eq!(five.window_minutes, Some(300));
    assert_eq!(weekly.resets_at, Some(t("2026-07-20T12:00:00Z")));
    assert_eq!(five.resets_at, Some(t("2026-07-17T18:00:00Z")));
}

#[test]
fn codex_burn_velocity_and_exhaustion_projection() {
    // Projections need a fresh trend: run "now" 10 minutes after the last
    // provider sample (11:00).
    let fresh = ScanContext {
        now: t("2026-07-17T11:10:00Z"),
        ..ctx()
    };
    let snap = codex("codex/basic/sessions").scan(&fresh);
    // Five-hour window: 10% -> 30% between 10:05 and 11:00 (55 min).
    let five = snap.quota_window(&QuotaWindowKind::FiveHour).unwrap();
    let burn = five.burn_per_hour.unwrap();
    assert!((burn - 20.0 / (55.0 / 60.0)).abs() < 0.01, "burn={burn}");
    // ~70% remaining at ~21.8%/h => empty early afternoon, before the
    // 18:00 reset -> at risk.
    match five.outlook() {
        QuotaOutlook::AtRisk {
            projected_exhaustion,
        } => {
            assert!(projected_exhaustion > t("2026-07-17T13:30:00Z"));
            assert!(projected_exhaustion < t("2026-07-17T15:30:00Z"));
        }
        other => panic!("expected AtRisk, got {other:?}"),
    }
    // Weekly window: 41.8 -> 42.0 over 55 min (~0.22%/h) => lasts past the
    // Monday reset.
    let weekly = snap.quota_window(&QuotaWindowKind::Weekly).unwrap();
    assert_eq!(weekly.outlook(), QuotaOutlook::Lasts);
}

#[test]
fn codex_stale_quota_trend_yields_no_projection() {
    // At 12:00 the newest sample is an hour old: showing a forecast from
    // it would be fabrication, so burn and exhaustion are absent while the
    // provider-reported percentage itself is still shown.
    let snap = codex("codex/basic/sessions").scan(&ctx());
    let five = snap.quota_window(&QuotaWindowKind::FiveHour).unwrap();
    assert!((five.used_percent - 30.0).abs() < 1e-9);
    assert_eq!(five.burn_per_hour, None);
    assert_eq!(five.projected_exhaustion, None);
    assert_eq!(five.outlook(), QuotaOutlook::Unknown);
}

#[test]
fn codex_reads_credits() {
    let snap = codex("codex/basic/sessions").scan(&ctx());
    let credits = snap.credits.as_ref().unwrap();
    assert!((credits.balance - 188.5).abs() < 1e-9);
}

#[test]
fn codex_forked_session_does_not_recount_inherited_history() {
    let snap = codex("codex/fork/sessions").scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    // First counter arrives with total(5800) != last(1050): only the
    // per-turn part counts, then the 10:50 delta on top.
    // input: 600 + 500, cached: 300 + 500, output: 150 + 200.
    assert_eq!(week.tokens.input, 1100);
    assert_eq!(week.tokens.cached_input, 800);
    assert_eq!(week.tokens.output, 350);
    assert_eq!(week.tokens.reasoning, 30);
}

#[test]
fn codex_survives_corrupt_and_truncated_lines() {
    let snap = codex("codex/corrupt/sessions").scan(&ctx());
    assert_eq!(snap.health.status, CollectorStatus::Degraded);
    assert_eq!(snap.health.parse_errors, 1); // garbage line only
    let week = snap.week.as_ref().unwrap();
    // The valid token_count still counts; the partial trailing line is
    // ignored without an error.
    assert_eq!(week.tokens.total(), 550);
}

#[test]
fn codex_weekly_only_never_masquerades_as_five_hour() {
    let snap = codex("codex/weekly_only/sessions").scan(&ctx());
    assert_eq!(snap.quota_windows.len(), 1);
    let w = &snap.quota_windows[0];
    assert_eq!(w.kind, QuotaWindowKind::Weekly);
    assert!(snap.quota_window(&QuotaWindowKind::FiveHour).is_none());
}

#[test]
fn codex_unknown_window_stays_unknown_and_visible() {
    let snap = codex("codex/unknown_window/sessions").scan(&ctx());
    assert_eq!(snap.quota_windows.len(), 1);
    let w = &snap.quota_windows[0];
    assert_eq!(w.kind, QuotaWindowKind::Unknown);
    assert_eq!(w.window_minutes, Some(90));
    assert_eq!(w.label(), "Window (90m)");
    assert!((w.used_percent - 55.0).abs() < 1e-9);
}

#[test]
fn codex_missing_directory_is_unavailable_not_error() {
    let mut collector = CodexCollector::with_dirs(vec![fixture("does/not/exist")]);
    let snap = collector.scan(&ctx());
    assert_eq!(snap.health.status, CollectorStatus::Unavailable);
    assert!(snap.quota_windows.is_empty());
    assert!(snap.sessions.is_empty());
}

#[test]
fn codex_context_window_utilization() {
    let snap = codex("codex/basic/sessions").scan(&ctx());
    let session = &snap.sessions[0];
    // Last request: input 1000 + output 150 of a 258400 window.
    assert_eq!(session.context_tokens, Some(1150));
    assert_eq!(session.context_window, Some(258_400));
    let pct = session.context_percent().unwrap();
    assert!(pct > 0.0 && pct < 1.0);
}

// --------------------------------------------------------------- Claude --

#[test]
fn claude_counts_each_api_request_once() {
    let snap = claude("claude/basic").scan(&ctx());
    assert_eq!(snap.health.status, CollectorStatus::Ok);
    let week = snap.week.as_ref().unwrap();
    // msg_01 appears on two lines (two content blocks) but counts once.
    // Totals across msg_01, msg_02, msg_03(sidechain), msg_04:
    assert_eq!(week.tokens.input, 19);
    assert_eq!(week.tokens.cache_creation, 3500);
    assert_eq!(week.tokens.cached_input, 15_100);
    assert_eq!(week.tokens.output, 540);
}

#[test]
fn claude_distinguishes_fable_from_other_models() {
    let snap = claude("claude/basic").scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    let fable = &week.by_model["claude-fable-5"];
    assert_eq!(fable.model.family, ModelFamily::ClaudeFable);
    assert_eq!(fable.model.display, "Fable 5");
    // msg_01 + sidechain msg_03.
    assert_eq!(fable.tokens.output, 380);
    let opus = &week.by_model["claude-opus-4-8"];
    assert_eq!(opus.model.family, ModelFamily::ClaudeOpus);
    assert_eq!(opus.tokens.output, 150);
}

#[test]
fn claude_preserves_unknown_models_and_token_categories() {
    let snap = claude("claude/basic").scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    // Future model: raw id kept, grouped as Other, never dropped.
    let nebula = &week.by_model["claude-nebula-9"];
    assert_eq!(nebula.model.family, ModelFamily::Other);
    assert_eq!(nebula.tokens.output, 10);
    // Unknown token category preserved for forward compatibility.
    let opus = &week.by_model["claude-opus-4-8"];
    assert_eq!(opus.tokens.other.get("mystery_tokens"), Some(&42));
}

#[test]
fn claude_ignores_synthetic_messages() {
    let snap = claude("claude/basic").scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    assert!(!week.by_model.contains_key("<synthetic>"));
}

#[test]
fn claude_fork_does_not_recount_inherited_lines() {
    let snap = claude("claude/fork").scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    // msg_10 exists in both sess-f1 and the forked sess-f2 but counts once;
    // msg_11 is new. output: 200 + 400.
    assert_eq!(week.tokens.output, 600);
    assert_eq!(week.tokens.input, 50);
}

#[test]
fn claude_sums_iterations_instead_of_undercounting_top_level() {
    // Multi-turn responses omit the largest turn from top-level usage;
    // summing iterations recovers it. msg_it1: out 236+8806+557=9599
    // (top-level says 793); msg_it2 single-turn: iterations == top level.
    let snap = claude("claude/iterations").scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    assert_eq!(week.tokens.output, 9599 + 50);
    assert_eq!(week.tokens.input, 128_680 + 10);
    assert_eq!(week.tokens.cache_creation, 2151 + 100);
    assert_eq!(week.tokens.cached_input, 253_894 + 200);
}

#[test]
fn claude_discovers_nested_subagent_and_workflow_transcripts() {
    // Transcripts live at three depths (main, subagents/, and
    // subagents/workflows/wf_*/); all must be counted.
    let snap = claude("claude/nested").scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    assert_eq!(week.tokens.output, 100 + 200 + 300);
    assert_eq!(week.tokens.input, 10 + 20 + 30);
}

#[test]
fn claude_reads_quota_from_local_cache() {
    let mut collector = ClaudeCollector::with_dirs_and_quota(
        vec![fixture("claude/basic")],
        fixture("claude/quota/claude.json"),
    );
    let snap = collector.scan(&ctx());
    assert!(snap.supports(Capability::ProviderQuota));
    assert!(snap.supports(Capability::ResetTimes));

    let five = snap.quota_window(&QuotaWindowKind::FiveHour).unwrap();
    assert!((five.used_percent - 27.0).abs() < 1e-9);
    assert_eq!(five.resets_at, Some(t("2026-07-17T14:40:00Z")));
    assert_eq!(five.captured_at, t("2026-07-17T11:30:00Z"));

    // Named weekly window plus the model-scoped weekly limit.
    let weeklies: Vec<_> = snap
        .quota_windows
        .iter()
        .filter(|w| w.kind == QuotaWindowKind::Weekly)
        .collect();
    assert_eq!(weeklies.len(), 2);
    let unscoped = weeklies.iter().find(|w| w.scope.is_none()).unwrap();
    assert!((unscoped.used_percent - 35.0).abs() < 1e-9);
    let scoped = weeklies.iter().find(|w| w.scope.is_some()).unwrap();
    assert!((scoped.used_percent - 52.0).abs() < 1e-9);
    assert_eq!(scoped.label(), "Weekly (Fable)");
    assert_eq!(scoped.resets_at, Some(t("2026-07-20T10:00:00Z")));

    // A single cache snapshot gives no burn trend — and never invents one.
    assert_eq!(five.burn_per_hour, None);
    assert_eq!(five.projected_exhaustion, None);
}

#[test]
fn claude_reports_no_quota_rather_than_inventing_it() {
    let snap = claude("claude/basic").scan(&ctx());
    assert!(!snap.supports(Capability::ProviderQuota));
    assert!(!snap.supports(Capability::Credits));
    assert!(snap.quota_windows.is_empty());
    assert!(snap.credits.is_none());
    assert!(snap.supports(Capability::LocalTokenUsage));
    assert!(snap.supports(Capability::ModelBreakdown));
}

#[test]
fn claude_survives_corrupt_and_truncated_lines() {
    let snap = claude("claude/corrupt").scan(&ctx());
    assert_eq!(snap.health.status, CollectorStatus::Degraded);
    assert_eq!(snap.health.parse_errors, 1);
    let week = snap.week.as_ref().unwrap();
    assert_eq!(week.tokens.output, 50);
}

#[test]
fn claude_session_metadata_is_content_free() {
    let snap = claude("claude/basic").scan(&ctx());
    let json = serde_json::to_string(&snap).unwrap();
    // Prompt bodies from fixture lines must never reach the snapshot.
    assert!(!json.contains("redacted synthetic fixture"));
    // Projects appear as basenames, not full paths.
    assert!(!json.contains("/home/user/projects"));
    assert!(json.contains("\"project\":\"gamma\""));
}

// ---------------------------------------------------- cross-provider ----

#[test]
fn snapshot_json_is_stable_across_identical_scans() {
    let a = serde_json::to_string(&codex("codex/basic/sessions").scan(&ctx())).unwrap();
    let b = serde_json::to_string(&codex("codex/basic/sessions").scan(&ctx())).unwrap();
    assert_eq!(a, b);
}

#[test]
fn rescan_without_changes_adds_nothing() {
    let mut collector = codex("codex/basic/sessions");
    let first = collector.scan(&ctx());
    let second = collector.scan(&ctx());
    assert_eq!(
        first.week.as_ref().unwrap().tokens,
        second.week.as_ref().unwrap().tokens
    );
    assert_eq!(first.sessions.len(), second.sessions.len());
}

#[test]
fn codex_incremental_ingestion_of_growing_file() {
    // Simulate a live session: scan, let the file grow (including a
    // partial trailing line), scan again. Only new complete lines count.
    let dir = std::env::temp_dir().join(format!("agentop-inc-test-{}", std::process::id()));
    let session_dir = dir.join("sessions/2026/07/17");
    std::fs::create_dir_all(&session_dir).unwrap();
    let path = session_dir.join("rollout-2026-07-17T10-00-00-live.jsonl");

    let meta = r#"{"timestamp":"2026-07-17T10:00:00.000Z","type":"session_meta","payload":{"id":"live-1","timestamp":"2026-07-17T10:00:00.000Z","cwd":"/home/user/projects/live","cli_version":"0.99.0"}}"#;
    let turn = r#"{"timestamp":"2026-07-17T10:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#;
    let tc1 = r#"{"timestamp":"2026-07-17T10:05:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":100,"reasoning_output_tokens":0,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":100,"reasoning_output_tokens":0,"total_tokens":1100},"model_context_window":258400},"rate_limits":null}}"#;
    let tc2 = r#"{"timestamp":"2026-07-17T10:10:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1600,"cached_input_tokens":0,"output_tokens":250,"reasoning_output_tokens":0,"total_tokens":1850},"last_token_usage":{"input_tokens":600,"cached_input_tokens":0,"output_tokens":150,"reasoning_output_tokens":0,"total_tokens":750},"model_context_window":258400},"rate_limits":null}}"#;

    std::fs::write(&path, format!("{meta}\n{turn}\n{tc1}\n")).unwrap();
    let mut collector = CodexCollector::with_dirs(vec![dir.join("sessions")]);
    let snap = collector.scan(&ctx());
    assert_eq!(snap.week.as_ref().unwrap().tokens.total(), 1100);

    // Append a complete line plus the beginning of the next one.
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    write!(f, "{tc2}\n{{\"timestamp\":\"2026-07-17T10:1").unwrap();
    drop(f);

    let snap = collector.scan(&ctx());
    let week = snap.week.as_ref().unwrap();
    // Cumulative 1850 total, counted exactly once (1100 + delta 750).
    assert_eq!(week.tokens.total(), 1850);
    assert_eq!(snap.health.parse_errors, 0);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn current_session_tokens_track_active_sessions_only() {
    // Fixture activity ends 11:00–11:50; with `now` at 12:00 nothing is
    // "active" (5-minute horizon), so current-session tokens are zero.
    let snap = codex("codex/basic/sessions").scan(&ctx());
    assert_eq!(snap.current_session_tokens.total(), 0);
    // With `now` right after the last event, the session is active.
    let near = ScanContext {
        now: t("2026-07-17T11:02:00Z"),
        ..ctx()
    };
    let snap = codex("codex/basic/sessions").scan(&near);
    assert_eq!(snap.current_session_tokens.total(), 3700);
}

use super::*;
use crate::collectors::store::OPENCODE_AUXILIARY;
use serde_json::json;
use std::time::Instant;

#[path = "../../../tests/support/opencode.rs"]
mod fixture;
use fixture::{Fixture, at, time, usage};

fn ctx() -> ScanContext {
    ScanContext {
        now: time(0),
        week_start: "2026-07-13T00:00:00Z".parse().unwrap(),
        week_end: "2026-07-20T00:00:00Z".parse().unwrap(),
        history_retention_minutes: 60,
    }
}

fn collector(path: &Path) -> OpenCodeCollector {
    OpenCodeCollector::from_config(&OpenCodeConfig {
        db_path: Some(path.to_owned()),
        ..Default::default()
    })
}

#[test]
fn maps_output_including_reasoning_once() {
    let fixture = Fixture::new();
    fixture.session("a", None, -10, -1);
    let mut data = usage("claude-opus-4-8", "claude-agent", -2, 4, 405);
    data["tokens"]["reasoning"] = json!(222);
    data["tokens"]["cache"] = json!({"read": 34435, "write": 997});
    data["tokens"]["unrecognized"] = json!(123);
    fixture.message("first", "a", -3, -2, data);
    let mut collector = collector(&fixture.path);
    collector.skip_upstreams.clear();
    let snap = collector.scan(&ctx());
    let tokens = snap.week.unwrap().tokens;
    assert_eq!(tokens.output, 627);
    assert_eq!(tokens.reasoning, 222);
    assert_eq!(tokens.total(), 4 + 34435 + 997 + 627);
    assert!(tokens.other.is_empty());
    assert_eq!(snap.sessions[0].context_tokens, Some(4 + 34435 + 997));
    assert_eq!(snap.sessions[0].context_window, None);
}

#[test]
fn skips_claude_agent_per_message_unless_list_is_empty() {
    let fixture = Fixture::new();
    fixture.session("mixed", None, -10, -1);
    fixture.session("mirrored", None, -10, -1);
    fixture.message(
        "1",
        "mixed",
        -5,
        -4,
        usage("gpt-6.2-sol", "openai", -4, 10, 2),
    );
    fixture.message(
        "2",
        "mixed",
        -4,
        -3,
        usage("claude-opus-4-8", "claude-agent", -3, 20, 3),
    );
    fixture.message(
        "3",
        "mirrored",
        -3,
        -2,
        usage("claude-opus-4-8", "claude-agent", -2, 30, 4),
    );
    let snap = collector(&fixture.path).scan(&ctx());
    assert_eq!(snap.week.unwrap().tokens.total(), 12);
    assert_eq!(snap.sessions.len(), 1);
    assert_eq!(snap.sessions[0].id, "mixed");
    assert_eq!(snap.sessions[0].model.as_ref().unwrap().raw, "gpt-6.2-sol");
    assert_eq!(snap.sessions[0].upstream.as_deref(), Some("openai"));

    let snap = OpenCodeCollector::from_config(&OpenCodeConfig {
        db_path: Some(fixture.path.clone()),
        skip_upstreams: vec![],
        ..Default::default()
    })
    .scan(&ctx());
    assert_eq!(snap.week.unwrap().tokens.total(), 69);
    assert_eq!(snap.sessions.len(), 2);
}

#[test]
fn in_flight_is_counted_only_once_completed() {
    let fixture = Fixture::new();
    fixture.session("a", None, -10, -1);
    fixture.message(
        "first",
        "a",
        -5,
        -5,
        json!({
            "model": {"providerID": "openai", "id": "gpt-6.2-sol"},
            "finish": null
        }),
    );
    let mut collector = collector(&fixture.path);
    let first = collector.scan(&ctx());
    assert!(first.sessions.is_empty());
    assert_eq!(first.week.unwrap().tokens.total(), 0);
    fixture.message(
        "first",
        "a",
        -5,
        -2,
        usage("gpt-6.2-sol", "openai", -2, 20, 22),
    );
    assert_eq!(collector.scan(&ctx()).week.unwrap().tokens.total(), 42);
    assert_eq!(collector.scan(&ctx()).week.unwrap().tokens.total(), 42);
}

#[test]
fn updates_record_only_the_saturating_delta_at_completion() {
    let fixture = Fixture::new();
    fixture.session("a", None, -10, -1);
    let mut collector = collector(&fixture.path);
    fixture.message(
        "first",
        "a",
        -5,
        -4,
        usage("gpt-6.2-sol", "openai", -4, 20, 22),
    );
    assert_eq!(collector.scan(&ctx()).week.unwrap().tokens.total(), 42);
    fixture.message(
        "first",
        "a",
        -5,
        -3,
        usage("gpt-6.2-sol", "openai", -4, 40, 2),
    );
    let snap = collector.scan(&ctx());
    let tokens = snap.week.unwrap().tokens;
    assert_eq!((tokens.input, tokens.output), (40, 22));
    let bucket = snap.history.iter().find(|b| b.at == time(-4)).unwrap();
    assert_eq!((bucket.input_tokens, bucket.output_tokens), (40, 22));
    assert_eq!(collector.scan(&ctx()).week.unwrap().tokens.total(), 62);
}

#[test]
fn counts_errors_only_with_usage() {
    let fixture = Fixture::new();
    fixture.session("a", None, -10, -1);
    for (id, output) in [("free", 0), ("billed", 9)] {
        let mut data = usage("gpt-6.2-sol", "openai", -2, 0, output);
        data["finish"] = json!("error");
        fixture.message(id, "a", -3, -2, data);
    }
    let snap = collector(&fixture.path).scan(&ctx());
    assert_eq!(snap.week.unwrap().tokens.total(), 9);
    assert_eq!(
        snap.health.message.as_deref(),
        Some("1 failed message without usage")
    );
}

#[test]
fn archived_session_still_counts() {
    let fixture = Fixture::new();
    fixture.session("archived", None, -10, -1);
    fixture
        .conn
        .execute("UPDATE session_v2 SET time_archived = ?1", [at(-9)])
        .unwrap();
    fixture.message(
        "first",
        "archived",
        -5,
        -4,
        usage("gpt-6.2-sol", "openai", -4, 3, 4),
    );
    let snap = collector(&fixture.path).scan(&ctx());
    assert_eq!(snap.week.unwrap().tokens.total(), 7);
    assert_eq!(snap.sessions[0].id, "archived");
}

#[test]
fn rolls_child_and_grandchild_usage_into_root() {
    let fixture = Fixture::new();
    fixture.session("root", None, -20, -1);
    fixture.session("child", Some("root"), -10, -2);
    fixture.session("grandchild", Some("child"), -9, -2);
    fixture.message(
        "first",
        "child",
        -5,
        -4,
        usage("gpt-6.2-sol", "openai", -4, 3, 4),
    );
    fixture.message(
        "second",
        "grandchild",
        -4,
        -3,
        usage("gpt-6.2-sol", "openai", -3, 5, 6),
    );
    let snap = collector(&fixture.path).scan(&ctx());
    let week = snap.week.unwrap();
    assert_eq!(week.tokens.total(), 18);
    assert_eq!(week.sessions, 1);
    assert_eq!(snap.sessions.len(), 1);
    let root = &snap.sessions[0];
    assert_eq!(root.id, "root");
    assert_eq!(root.project.as_deref(), Some("root"));
    assert_eq!(root.started_at, Some(time(-20)));
    assert_eq!(root.last_activity, Some(time(-3)));
}

#[test]
fn models_split_and_latest_completion_wins_even_if_older_message_was_updated_last() {
    let fixture = Fixture::new();
    fixture.session("a", None, -10, -1);
    fixture.message(
        "first",
        "a",
        -5,
        -1,
        usage("gpt-6.2-sol", "openai", -4, 3, 4),
    );
    fixture.message(
        "second",
        "a",
        -4,
        -3,
        usage("gemini-9", "openrouter", -3, 5, 6),
    );
    let snap = collector(&fixture.path).scan(&ctx());
    let session = &snap.sessions[0];
    assert_eq!(session.tokens_by_model["gpt-6.2-sol"].total(), 7);
    assert_eq!(session.tokens_by_model["gemini-9"].total(), 11);
    assert_eq!(session.model.as_ref().unwrap().raw, "gemini-9");
    assert_eq!(session.upstream.as_deref(), Some("openrouter"));
    let mut json = serde_json::to_value(session).unwrap();
    assert_eq!(json["upstream"], "openrouter");
    json.as_object_mut().unwrap().remove("upstream");
    let old: crate::domain::SessionUsage = serde_json::from_value(json).unwrap();
    assert_eq!(old.upstream, None);
}

#[test]
fn doctor_reports_discovery_and_48h_message_counts_by_upstream() {
    let fixture = Fixture::new();
    fixture.session("a", None, -10, -1);
    fixture.session("old", None, -3 * 24 * 60, -3 * 24 * 60);
    fixture.message(
        "older",
        "old",
        -3 * 24 * 60,
        -3 * 24 * 60,
        usage("gpt-6.2-sol", "openai", -3 * 24 * 60, 90, 10),
    );
    fixture.message(
        "counted",
        "a",
        -9,
        -8,
        usage("gpt-6.2-sol", "openai", -8, 10, 20),
    );
    fixture.message(
        "skipped",
        "a",
        -5,
        -4,
        usage("claude-opus-4-8", "claude-agent", -4, 100, 200),
    );
    let mut failed = usage("gpt-6.2-sol", "openai", -3, 0, 0);
    failed["finish"] = json!("error");
    fixture.message("failed", "a", -4, -3, failed);
    let (snapshot, details) = collector(&fixture.path).scan_for_doctor(&ctx());
    assert_eq!(snapshot.week.unwrap().tokens.total(), 130);
    assert!(details.found && details.readable);
    assert_eq!(details.version.as_deref(), Some("2.3.2"));
    assert_eq!(
        details.migration.as_deref(),
        Some("20261116213417_automation_active_index")
    );
    let usage = details.usage.unwrap();
    assert_eq!(usage.sessions, 1);
    assert_eq!(
        (
            usage.counted["openai"].messages,
            usage.counted["openai"].tokens
        ),
        (1, 30)
    );
    assert_eq!(
        (
            usage.skipped["claude-agent"].messages,
            usage.skipped["claude-agent"].tokens
        ),
        (1, 300)
    );
    assert_eq!(usage.failed_messages, 1);
}

#[test]
fn counts_only_safe_residuals_against_lifetime_messages() {
    let fixture = Fixture::new();
    let historical = -7 * 24 * 60;
    for id in ["safe", "mixed"] {
        fixture.session(id, None, historical - 2, -1);
        fixture.message(id, id, -6, -4, usage("gpt-6.2-sol", "openai", -4, 3, 4));
    }
    fixture.conn.execute(
        "UPDATE session_v2 SET tokens_input=20, tokens_output=3, tokens_reasoning=5, tokens_cache_write=2",
        [],
    ).unwrap();
    fixture.message(
        "s-old",
        "safe",
        historical - 1,
        historical,
        usage("gpt-6.2-sol", "openai", historical, 10, 1),
    );
    fixture.message(
        "m-old",
        "mixed",
        historical - 1,
        historical,
        usage("claude-opus-4-8", "claude-agent", historical, 10, 1),
    );
    let mut collector = collector(&fixture.path);
    let (snap, doctor) = collector.scan_for_doctor(&ctx());
    let week = snap.week.unwrap();
    let auxiliary = &week.by_model[OPENCODE_AUXILIARY].tokens;
    assert_eq!(
        (auxiliary.input, auxiliary.output, auxiliary.cache_creation),
        (7, 5, 2)
    );
    assert_eq!(auxiliary.reasoning, 5);
    assert_eq!(week.tokens.total(), 7 + 7 + 14);
    let mixed = snap.sessions.iter().find(|s| s.id == "mixed").unwrap();
    assert!(!mixed.tokens_by_model.contains_key(OPENCODE_AUXILIARY));
    let withheld = mixed.auxiliary_not_attributed.as_ref().unwrap();
    assert_eq!(withheld.tokens, 14);
    assert!(withheld.may_be_counted_under_claude);
    let safe = snap.sessions.iter().find(|s| s.id == "safe").unwrap();
    assert!(safe.auxiliary_not_attributed.is_none());
    assert_eq!(safe.tokens_by_model[OPENCODE_AUXILIARY].total(), 14);
    assert_eq!(safe.last_activity, Some(time(-4)));
    assert_eq!(safe.model.as_ref().unwrap().raw, "gpt-6.2-sol");
    assert_eq!(
        snap.history
            .iter()
            .find(|b| b.at == time(-1))
            .unwrap()
            .output_tokens,
        5
    );
    let tallies = doctor.usage.unwrap();
    assert_eq!(tallies.auxiliary_counted, 14);
    assert_eq!(tallies.auxiliary_skipped, 14);
    assert_eq!(tallies.auxiliary_unknown, 0);
    // Session metadata/tallies can change without a new message timestamp.
    fixture
        .conn
        .execute(
            "UPDATE session_v2 SET tokens_reasoning=1 WHERE id='safe'",
            [],
        )
        .unwrap();
    assert_eq!(collector.scan(&ctx()).week.unwrap().tokens.total(), 24);
    fixture.message(
        "s-late",
        "safe",
        -5,
        0,
        usage("claude-opus-4-8", "claude-agent", -3, 0, 0),
    );
    let snap = collector.scan(&ctx());
    assert_eq!(snap.week.unwrap().tokens.total(), 14);
}

#[test]
fn unknown_upstream_blocks_residual_without_a_claude_hint() {
    let fixture = Fixture::new();
    fixture.session("known", None, -10, -1);
    fixture.message(
        "m1",
        "known",
        -9,
        -8,
        usage("gpt-6.2-sol", "openai", -8, 10, 2),
    );
    fixture.message("m2", "known", -2, -1, json!({"finish":null}));
    fixture
        .conn
        .execute("UPDATE session_v2 SET tokens_input=20, tokens_output=2", [])
        .unwrap();
    let (snap, doctor) = collector(&fixture.path).scan_for_doctor(&ctx());
    assert_eq!(snap.week.unwrap().tokens.total(), 12);
    let withheld = snap.sessions[0].auxiliary_not_attributed.as_ref().unwrap();
    assert_eq!(withheld.tokens, 10);
    assert!(!withheld.may_be_counted_under_claude);
    assert_eq!(doctor.usage.unwrap().auxiliary_unknown, 10);
}

#[test]
fn shared_plan_requires_counted_upstream_and_matching_oauth_metadata() {
    let fixture = Fixture::new();
    fixture.session("a", None, -10, -1);
    fixture.message("m1", "a", -9, -8, usage("gpt-6.2-sol", "openai", -8, 10, 2));
    let mut collector = collector(&fixture.path);
    assert!(collector.scan(&ctx()).shared_plans.is_empty()); // no credential table

    fixture
        .conn
        .execute_batch(
            "CREATE TABLE credential (integration_id TEXT, value TEXT);
         INSERT INTO credential VALUES
           ('openai', '{\"type\":\"api\",\"methodID\":\"chatgpt-token-sharing\"}'),
           ('anthropic', '{\"type\":\"oauth\",\"access\":\"synthetic-credential-not-read\"}');",
        )
        .unwrap();
    let snap = collector.scan(&ctx());
    assert!(snap.shared_plans.is_empty()); // API key and no counted Anthropic
    assert!(snap.quota_windows.is_empty());
    fixture.conn.execute(
        "UPDATE credential SET value=?1 WHERE integration_id='openai'",
        [r#"{"type":"oauth","methodID":"not-chatgpt","access":"synthetic-credential-not-read"}"#],
    ).unwrap();
    assert!(collector.scan(&ctx()).shared_plans.is_empty());
    fixture.conn.execute(
        "UPDATE credential SET value=?1 WHERE integration_id='openai'",
        [r#"{"type":"oauth","methodID":"chatgpt-token-sharing","access":"synthetic-credential-not-read"}"#],
    ).unwrap();
    let snap = collector.scan(&ctx());
    assert_eq!(snap.shared_plans, [SharedPlan::ChatGpt]);
    assert!(snap.quota_windows.is_empty());
    assert!(!snap.supports(Capability::ProviderQuota));
    fixture.message(
        "m2",
        "a",
        -3,
        -2,
        usage("claude-opus-4-8", "anthropic", -2, 5, 4),
    );
    let (snap, doctor) = collector.scan_for_doctor(&ctx());
    assert_eq!(snap.shared_plans, [SharedPlan::ChatGpt, SharedPlan::Claude]);
    assert_eq!(snap.shared_plans, doctor.shared_plans);
    assert_eq!(snap.week.as_ref().unwrap().tokens.total(), 21);
    let json = serde_json::to_string(&snap).unwrap();
    assert!(json.contains(r#""shared_plans":["chatgpt","claude"]"#));
    assert!(!json.contains("synthetic-credential-not-read"));
    assert!(!json.contains("chatgpt-token-sharing"));
    let excluded = OpenCodeCollector::from_config(&OpenCodeConfig {
        db_path: Some(fixture.path.clone()),
        skip_upstreams: vec!["openai".into()],
        ..Default::default()
    })
    .scan(&ctx());
    assert_eq!(excluded.shared_plans, [SharedPlan::Claude]);
}

#[test]
fn unreadable_auth_metadata_degrades_without_losing_observed_usage() {
    let fixture = Fixture::new();
    fixture.session("a", None, -10, -1);
    fixture.message("m1", "a", -9, -8, usage("gpt-6.2-sol", "openai", -8, 10, 2));
    fixture
        .conn
        .execute_batch(
            "CREATE TABLE credential (integration_id TEXT, value TEXT);
         INSERT INTO credential VALUES ('openai', 'invalid-synthetic-value');",
        )
        .unwrap();
    let snap = collector(&fixture.path).scan(&ctx());
    assert_eq!(snap.health.status, CollectorStatus::Degraded);
    assert_eq!(snap.week.unwrap().tokens.total(), 12);
    assert!(snap.shared_plans.is_empty());
    assert_eq!(
        snap.health.message.as_deref(),
        Some("could not read OpenCode quota-plan metadata")
    );
}

#[test]
fn schema_drift_is_unavailable_and_corrupt_file_is_degraded() {
    for ddl in [
        "DROP TABLE session_message",
        "ALTER TABLE session_message DROP COLUMN time_updated",
    ] {
        let fixture = Fixture::new();
        fixture.conn.execute_batch(ddl).unwrap();
        let snap = collector(&fixture.path).scan(&ctx());
        assert_eq!(snap.health.status, CollectorStatus::Unavailable);
        assert_eq!(
            snap.health.message.as_deref(),
            Some(
                "unsupported OpenCode schema (latest migration 20261116213417_automation_active_index)"
            )
        );
        assert!(snap.week.is_none());
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-db");
    std::fs::write(&path, b"plain text").unwrap();
    let snap = collector(&path).scan(&ctx());
    assert_eq!(snap.health.status, CollectorStatus::Degraded);
    assert!(snap.week.is_none());
    assert_eq!(
        collector(&dir.path().join("missing.db"))
            .scan(&ctx())
            .health
            .status,
        CollectorStatus::Unavailable
    );
}

#[test]
fn locked_database_degrades_within_busy_timeout() {
    let fixture = Fixture::new();
    // Normal WAL writes don't lock readers; force contention with an
    // exclusive write transaction in rollback-journal mode.
    fixture
        .conn
        .execute_batch("PRAGMA journal_mode=DELETE; BEGIN EXCLUSIVE;")
        .unwrap();
    let mut collector = collector(&fixture.path);
    let start = Instant::now();
    let snap = collector.scan(&ctx());
    assert!(start.elapsed() < BUSY_TIMEOUT + Duration::from_millis(250));
    assert_eq!(snap.health.status, CollectorStatus::Degraded);
    assert_eq!(
        snap.health.message.as_deref(),
        Some("OpenCode database is locked")
    );
    assert!(snap.week.is_none());
    fixture.conn.execute_batch("ROLLBACK;").unwrap();
    assert_eq!(collector.scan(&ctx()).health.status, CollectorStatus::Ok);
}

#[test]
fn maps_known_effort_only_and_does_not_inherit() {
    let fixture = Fixture::new();
    fixture.session("a", None, -30, -1);
    let mut collector = collector(&fixture.path);
    for (i, (variant, expected)) in [
        (json!("minimal"), Some("minimal")),
        (json!("low"), Some("low")),
        (json!("medium"), Some("medium")),
        (json!("high"), Some("high")),
        (json!("xhigh"), Some("xhigh")),
        (json!("default"), None),
        (json!("future"), None),
        (json!(42), None),
        (json!(null), None),
    ]
    .into_iter()
    .enumerate()
    {
        let minute = i as i64 - 20;
        let mut data = usage("gpt-6.2-sol", "openai", minute, 2, 3);
        data["model"]["variant"] = variant;
        fixture.message(&format!("turn-{i}"), "a", minute, minute, data);
        let snap = collector.scan(&ctx());
        assert_eq!(snap.sessions[0].reasoning_effort.as_deref(), expected);
    }
}

#[test]
fn sql_projections_stay_on_the_metadata_allowlist() {
    for sql in [MESSAGES, RESIDUALS, SESSION] {
        for forbidden in [
            "content",
            "session_part",
            "title",
            "diff",
            "credential",
            "cost",
            "SELECT *",
        ] {
            assert!(!sql.contains(forbidden), "{forbidden} in usage SQL");
        }
        for (i, _) in sql.match_indices("data") {
            assert!(sql[..i].ends_with("json_extract(m."));
            let path = sql[i..].split('\'').nth(1).unwrap();
            assert!(
                matches!(
                    path,
                    "$.model.providerID"
                        | "$.model.id"
                        | "$.model.variant"
                        | "$.time.completed"
                        | "$.finish"
                        | "$.tokens.input"
                        | "$.tokens.output"
                        | "$.tokens.reasoning"
                        | "$.tokens.cache.read"
                        | "$.tokens.cache.write"
                ),
                "unexpected path {path}"
            );
        }
    }
    assert!(CREDENTIAL_PLANS.starts_with("SELECT integration_id,"));
    assert!(!CREDENTIAL_PLANS.contains("SELECT *"));
    let auth_paths: Vec<_> = CREDENTIAL_PLANS
        .match_indices("value")
        .map(|(i, _)| {
            assert!(CREDENTIAL_PLANS[..i].ends_with("json_extract("));
            CREDENTIAL_PLANS[i..].split('\'').nth(1).unwrap()
        })
        .collect();
    assert_eq!(auth_paths, ["$.type", "$.methodID"]);
}

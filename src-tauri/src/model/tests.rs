//! Model-layer tests, split from `model.rs` (架构审查Ⅶ候选 A3 followed the
//! repo rule that new tests live in a dedicated test-module file). The
//! cross-grain mapping tests at the bottom pin the one seam between
//! `SessionFilter` and `UsageFilter`.

use super::*;
use rust_decimal::Decimal;
use std::str::FromStr;

#[test]
fn day_from_timestamp_utc_bucket() {
    assert_eq!(
        UsageRecord::day_from_timestamp("2026-07-13T16:55:22.467Z"),
        "2026-07-13"
    );
}

#[test]
fn day_from_timestamp_garbage_falls_back_to_prefix() {
    // Unparseable but ≥10 chars ⇒ first 10 chars as the day bucket.
    assert_eq!(
        UsageRecord::day_from_timestamp("garbage-input-here"),
        "garbage-in"
    );
    // <10 chars ⇒ the explicit fallback sentinel.
    assert_eq!(UsageRecord::day_from_timestamp("short"), "0000-00-00");
}

#[test]
fn token_total_sums_four_buckets() {
    let t = TokenCounts {
        input: 100,
        output: 50,
        cache_creation: 10,
        cache_read: 90,
    };
    assert_eq!(t.total(), 250);
}

#[test]
fn token_cache_hit_rate() {
    let t = TokenCounts {
        input: 100,
        output: 50,
        cache_creation: 10,
        cache_read: 90,
    };
    assert!((t.cache_hit_rate() - 90.0 / 200.0).abs() < 1e-9);
    // Nothing cacheable ⇒ 0.
    let z = TokenCounts {
        input: 0,
        output: 5,
        cache_creation: 0,
        cache_read: 0,
    };
    assert_eq!(z.cache_hit_rate(), 0.0);
}

#[test]
fn cost_breakdown_total_is_bucket_sum() {
    let cb = CostBreakdown::from_buckets(
        Decimal::from_str("1.0").unwrap(),
        Decimal::from_str("2.0").unwrap(),
        Decimal::from_str("0.5").unwrap(),
        Decimal::from_str("0.5").unwrap(),
    );
    assert_eq!(cb.total_usd, Decimal::from_str("4.0").unwrap());
}

#[test]
fn usage_record_carries_new_per_call_fields() {
    let r = UsageRecord {
        uuid: "u1".into(),
        timestamp: "2026-07-21T10:00:00Z".into(),
        day: "2026-07-21".into(),
        model: "glm-5.2".into(),
        pricing_model: "glm-5.2".into(),
        source: "claude_code".into(),
        session_id: "session-abc".into(),
        device_id: "abc123def456".into(),
        tokens: TokenCounts::default(),
        server_tool_use: ServerToolUse::default(),
        stop_reason: "tool_use".into(),
        service_tier: "standard".into(),
        iterations: 3,
        cost: CostBreakdown::default(),
    };
    assert_eq!(r.stop_reason, "tool_use");
    assert_eq!(r.service_tier, "standard");
    assert_eq!(r.iterations, 3);
    assert_eq!(r.session_id, "session-abc");
}

#[test]
fn usage_record_session_id_defaults_empty_when_absent_in_jsonl() {
    // An older Artifact line (pre-session) lacks `session_id`. It must
    // deserialize with an empty default rather than fail — the column was
    // added after the initial schema, and peers may still carry old lines.
    let json = r#"{"uuid":"u1","timestamp":"2026-07-21T10:00:00Z","day":"2026-07-21","model":"glm-5.2","pricing_model":"glm-5.2","source":"claude_code","device_id":"abc123def456","tokens":{"input":0,"output":0,"cache_creation":0,"cache_read":0},"server_tool_use":{"web_search":0,"web_fetch":0},"stop_reason":"","service_tier":"","iterations":0,"cost":{"input_usd":"0","output_usd":"0","cache_read_usd":"0","cache_creation_usd":"0","total_usd":"0"}}"#;
    let r: UsageRecord = serde_json::from_str(json).unwrap();
    assert_eq!(r.session_id, "", "absent session_id ⇒ empty default");
}

#[test]
fn session_types_roundtrip() {
    let sys = SessionSystemData {
        id: "s1".into(),
        source: "claude_code".into(),
        project_dir: "/proj".into(),
        title_orig: "Hello".into(),
        started_at: "2026-08-01T10:00:00Z".into(),
        last_active_at: "2026-08-01T11:00:00Z".into(),
        agent_type: String::new(),
        parent_session_id: String::new(),
    };
    let s: SessionSystemData = serde_json::from_str(&serde_json::to_string(&sys).unwrap()).unwrap();
    assert_eq!(s, sys);
}

#[test]
fn snapshot_meta_parent_link_roundtrips_and_defaults() {
    // The parent link rides today's meta line unchanged...
    let meta = SessionSnapshotMeta {
        v: 1,
        id: "agent-x".into(),
        source: "claude_code".into(),
        project_dir: "/proj".into(),
        title_orig: "Task".into(),
        started_at: "2026-08-01T10:00:00Z".into(),
        last_active_at: "2026-08-01T11:00:00Z".into(),
        agent_type: "Explore".into(),
        parent_session_id: "main-1".into(),
        favorited: true,
        synced_group_id: String::new(),
    };
    let back: SessionSnapshotMeta =
        serde_json::from_str(&serde_json::to_string(&meta).unwrap()).unwrap();
    assert_eq!(back.parent_session_id, "main-1");
    // ...and a pre-field snapshot (written before #90) still parses,
    // defaulting to no parent — no SESSION_SNAPSHOT_VERSION bump needed.
    let legacy = r#"{"v":1,"id":"agent-x","source":"claude_code","project_dir":"/proj","title_orig":"Task","started_at":"2026-08-01T10:00:00Z","last_active_at":"2026-08-01T11:00:00Z","agent_type":"Explore","favorited":true,"synced_group_id":""}"#;
    let old: SessionSnapshotMeta = serde_json::from_str(legacy).unwrap();
    assert_eq!(old.parent_session_id, "", "absent parent => empty default");
    assert_eq!(old.agent_type, "Explore");
}

#[test]
fn session_message_roundtrips_and_skips_none_extras() {
    let m = SessionMessage {
        uuid: "e1".into(),
        session_id: "s1".into(),
        role: SessionMessageRole::Assistant,
        ts: "2026-08-01T10:00:00Z".into(),
        model: Some("glm-5.2".into()),
        name: None,
        content: "hi".into(),
    };
    let json = serde_json::to_string(&m).unwrap();
    let back: SessionMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back, m);
    // `name: None` is skipped on serialize (skip_serializing_if).
    assert!(!json.contains("\"name\""));
}

#[test]
fn turn_duration_roundtrips() {
    let td = TurnDuration {
        uuid: "t1".into(),
        timestamp: "2026-07-21T10:00:00Z".into(),
        day: "2026-07-21".into(),
        session_id: "sess-abc".into(),
        device_id: "abc123def456".into(),
        duration_ms: 209_499,
    };
    let json = serde_json::to_string(&td).unwrap();
    let back: TurnDuration = serde_json::from_str(&json).unwrap();
    assert_eq!(back, td);
}

/// An old `turns-*.jsonl` artifact line predating the `session_id` field
/// must parse (defaulting to "") rather than fail — peers may carry old
/// lines, and a "" session id simply buckets the turn into the unknown
/// project (no session row resolves).
#[test]
fn turn_duration_without_session_id_parses_to_unknown() {
    let json = r#"{"uuid":"t1","timestamp":"2026-07-21T10:00:00Z","day":"2026-07-21","device_id":"abc123def456","duration_ms":209499}"#;
    let td: TurnDuration = serde_json::from_str(json).unwrap();
    assert_eq!(td.uuid, "t1");
    assert_eq!(td.session_id, "", "absent session_id ⇒ empty default");
}

// ---- Model-key normalization sub-steps ----

#[test]
fn strip_provider_prefix_keeps_tail_after_last_slash() {
    assert_eq!(strip_provider_prefix("openai/gpt-5.4"), "gpt-5.4");
    assert_eq!(strip_provider_prefix("a/b/c"), "c");
    // No slash → unchanged.
    assert_eq!(strip_provider_prefix("gpt-5.4"), "gpt-5.4");
}

#[test]
fn strip_brackets_drops_context_window_tag() {
    assert_eq!(strip_brackets("glm-5.2[1m]"), "glm-5.2");
    // Trailing whitespace before the bracket is trimmed.
    assert_eq!(strip_brackets("glm-5.2 [1m]"), "glm-5.2");
    // No bracket → unchanged.
    assert_eq!(strip_brackets("gpt-5.4"), "gpt-5.4");
}

#[test]
fn strip_iso_date_suffix_matches_only_dashed_iso_form() {
    assert_eq!(strip_iso_date_suffix("gpt-5.4-2026-03-05"), "gpt-5.4");
    assert_eq!(
        strip_iso_date_suffix("gpt-5.4-pro-2026-03-05"),
        "gpt-5.4-pro"
    );
    // Compact 8-digit form is NOT the ISO step's concern.
    assert_eq!(
        strip_iso_date_suffix("gpt-5.4-20260305"),
        "gpt-5.4-20260305"
    );
    // Non-date tail → unchanged.
    assert_eq!(strip_iso_date_suffix("gpt-5.2-codex"), "gpt-5.2-codex");
}

#[test]
fn strip_compact_date_suffix_matches_only_eight_digit_form() {
    assert_eq!(
        strip_compact_date_suffix("claude-3-5-haiku-20241022"),
        "claude-3-5-haiku"
    );
    assert_eq!(strip_compact_date_suffix("gpt-5.4-20260305"), "gpt-5.4");
    // ISO form (dashes inside) is NOT the compact step's concern.
    assert_eq!(
        strip_compact_date_suffix("gpt-5.4-2026-03-05"),
        "gpt-5.4-2026-03-05"
    );
    // Non-date tail → unchanged.
    assert_eq!(strip_compact_date_suffix("gpt-5.2-codex"), "gpt-5.2-codex");
}

// ---- Model-key normalization entry points ----

#[test]
fn normalize_model_key_applies_the_full_superset() {
    // Lowercase + prefix + ISO date.
    assert_eq!(normalize_model_key("OPENAI/GPT-5.4-2026-03-05"), "gpt-5.4");
    // Lowercase + prefix + compact date.
    assert_eq!(normalize_model_key("openai/gpt-5.4-20260305"), "gpt-5.4");
    // Lowercase only.
    assert_eq!(normalize_model_key("GLM-4.6"), "glm-4.6");
    // ISO date with a version token before it.
    assert_eq!(normalize_model_key("gpt-5.4-pro-2026-03-05"), "gpt-5.4-pro");
    // Compact date after a versioned name.
    assert_eq!(
        normalize_model_key("claude-opus-4-6-20260206"),
        "claude-opus-4-6"
    );
    // No prefix/date/brackets → only lowercased.
    assert_eq!(normalize_model_key("gpt-5.2-codex"), "gpt-5.2-codex");
    assert_eq!(normalize_model_key("o3"), "o3");
    // Brackets are stripped too: a no-op for Codex today, but the superset
    // keeps the rule so a future bracketed Codex name still matches.
    assert_eq!(normalize_model_key("openai/gpt-5.4[1m]"), "gpt-5.4");
}

// ---- Cross-grain filter mapping (架构审查Ⅶ候选 A3) ----
//
// The one seam between the two filter shapes: the five shared facets (time /
// device / model / source) carried both ways by exhaustive struct literals —
// a new field on either filter type fails the literals to compile, so a sixth
// shared facet cannot be wired in one direction only (the old
// `..Default::default()` reverse mapping let the project dropdown silently
// miss a new facet's narrowing).

/// SessionFilter → UsageFilter：五个共享轴逐字段搬运，`project` 恒 `None`
/// （桶的项目身份 = NOT EXISTS，不是筛选）。
#[test]
fn to_usage_grain_maps_the_shared_axes_and_drops_project() {
    let f = SessionFilter {
        from_ts: Some("2026-08-01T00:00:00Z".into()),
        to_ts: Some("2026-08-27T00:00:00Z".into()),
        model: Some("glm-5.2".into()),
        source: Some("claude_code".into()),
        device_scope: Some("dev".into()),
        project: Some(UNKNOWN_PROJECT.into()),
        ..Default::default()
    };
    let u = f.to_usage_grain();
    assert_eq!(u.from_ts, f.from_ts);
    assert_eq!(u.to_ts, f.to_ts);
    assert_eq!(u.model, f.model);
    assert_eq!(u.source, f.source);
    assert_eq!(u.device_scope, f.device_scope);
    assert!(u.project.is_none(), "项目不映射——桶自身即项目定义");
}

/// UsageFilter → SessionFilter：同一组共享轴反向搬运，`project` 与全部
/// sessions-only facet（favorited / groups / search）显式 `None`——项目下拉
/// 候选不被自己的产品轴收窄，sessions-only 语义不跨粒泄漏。
#[test]
fn to_session_grain_maps_the_shared_axes_and_drops_sessions_only_facets() {
    let f = UsageFilter {
        from_ts: Some("2026-08-01T00:00:00Z".into()),
        to_ts: Some("2026-08-27T00:00:00Z".into()),
        model: Some("glm-5.2".into()),
        source: Some("claude_code".into()),
        device_scope: Some("dev".into()),
        project: Some(UNKNOWN_PROJECT.into()),
    };
    let s = f.to_session_grain();
    assert_eq!(s.from_ts, f.from_ts);
    assert_eq!(s.to_ts, f.to_ts);
    assert_eq!(s.model, f.model);
    assert_eq!(s.source, f.source);
    assert_eq!(s.device_scope, f.device_scope);
    assert!(s.project.is_none());
    assert!(s.favorited.is_none());
    assert!(s.local_group_id.is_none());
    assert!(s.synced_group_id.is_none());
    assert!(s.search.is_none());
}

/// 对偶断言：两个方向搬运的是同一字段集。置满共享轴后往返
/// to_usage_grain → to_session_grain 逐位保持——与上面两个单方向测试共同
/// 钉死「恰好这五个字段跨粒」：加第六个共享 facet 而只接一个方向时，
/// 穷举字面量先红、往返再红，双保险。
#[test]
fn cross_grain_round_trip_preserves_exactly_the_shared_axes() {
    let f = SessionFilter {
        from_ts: Some("2026-08-01T00:00:00Z".into()),
        to_ts: Some("2026-08-27T00:00:00Z".into()),
        model: Some("glm-5.2".into()),
        source: Some("claude_code".into()),
        device_scope: Some("dev".into()),
        ..Default::default()
    };
    let back = f.to_usage_grain().to_session_grain();
    assert_eq!(back.from_ts, f.from_ts);
    assert_eq!(back.to_ts, f.to_ts);
    assert_eq!(back.model, f.model);
    assert_eq!(back.source, f.source);
    assert_eq!(back.device_scope, f.device_scope);
    // Sessions-only facets cannot survive the trip through the usage grain.
    assert!(back.project.is_none());
    assert!(back.search.is_none());
}

// ---- project_identity: worktree suffix collapses to the parent project ----
// (moved from session.rs along with the function — 架构审查Ⅶ候选 A3)

// ---- project_identity: worktree suffix collapses to the parent project ----

#[test]
fn project_identity_collapses_windows_worktree_suffix() {
    // The real-world shape this rule was derived from (issue #84): a
    // subagent/parallel session launched in a Claude Code worktree.
    assert_eq!(
        project_identity("D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b"),
        "D:\\Project\\O_CC_One"
    );
}

#[test]
fn project_identity_collapses_unix_worktree_suffix() {
    // A Unix peer's cwd lands in the same cross-device store.
    assert_eq!(
        project_identity("/home/me/proj/.claude/worktrees/agent-ff"),
        "/home/me/proj"
    );
}

#[test]
fn project_identity_no_worktree_segment_is_unchanged() {
    // Ordinary launch dirs — including a project that merely CONTAINS a
    // `.claude` dir (without the `worktrees` child) — pass through.
    assert_eq!(
        project_identity("D:\\Project\\O_CC_One"),
        "D:\\Project\\O_CC_One"
    );
    assert_eq!(project_identity("/home/me/proj"), "/home/me/proj");
    assert_eq!(project_identity("D:\\foo\\.claude"), "D:\\foo\\.claude");
    // A directory whose name merely ends in `.claude` is NOT the segment.
    assert_eq!(
        project_identity("D:\\foo\\my.claude\\worktrees\\x"),
        "D:\\foo\\my.claude\\worktrees\\x"
    );
    assert_eq!(project_identity(""), "");
}

#[test]
fn project_identity_empty_parent_keeps_raw_form() {
    // A bare relative worktree path would truncate to nothing; keeping the
    // raw string avoids degrading the row to the empty no-project bucket.
    assert_eq!(
        project_identity(".claude\\worktrees\\agent-x"),
        ".claude\\worktrees\\agent-x"
    );
}

#[test]
fn project_identity_trailing_separator_and_nested_forms() {
    // Trailing separator: the tail empty component changes nothing.
    assert_eq!(project_identity("/p/.claude/worktrees/agent-x/"), "/p");
    // First segment wins when (pathologically) two appear.
    assert_eq!(
        project_identity("/p/.claude/worktrees/x/.claude/worktrees/y"),
        "/p"
    );
}

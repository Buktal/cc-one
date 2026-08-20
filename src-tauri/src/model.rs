//! Core domain model for the rebuilt cc one.
//!
//! Two grains (re-derivation 2026-07-21):
//!   - [`UsageRecord`]: one model API call (per-call). The unit a parser
//!     emits, the Local Store stores, and one JSONL line serializes.
//!   - [`TurnDuration`]: one turn's wall-clock (per-turn), sourced from the
//!     `system/turn_duration` event. Separate from per-call records because a
//!     turn spans multiple API calls.
//!
//! Boundary type rules: no pointer-sized ints cross the Rust→JS boundary.
//! Token counts are `u32`; timestamps cross as ISO8601 strings; cost crosses as
//! `f64` (display-only on the JS side — JS never recomputes cost), while cost
//! is kept internally as `rust_decimal::Decimal` and stored as TEXT in SQLite.
//!
//! Organized by domain into submodules, all re-exported here so callers keep
//! using `crate::model::Type`:
//!   - [`device`] — device identity artifact + read-side device/run-mode types.
//!   - [`session`] — session, transcript, and snapshot types.
//!   - [`usage`] — per-call / per-turn usage records, DTOs, and pricing.
//!
//! The model-key normalizer lives in this file (single source of truth).

mod device;
mod provider;
mod session;
mod usage;

pub use device::*;
pub use provider::*;
pub use session::*;
pub use usage::*;

// ---- Model-key normalization (single source of truth) ----
//
// One canonical form for a model key, applied at every site that matches a
// model name against the pricing book: parsers normalize the raw names they
// parse (e.g. Codex's `openai/gpt-5.4-2026-03-05`), the pricing book
// normalizes both its table keys and its lookup candidates, and ingest
// normalizes the rebill key. One rule everywhere, so a model can never match
// in one place and miss in another. Built from orthogonal sub-steps that are
// each a no-op when their pattern is absent.

/// Strip a `provider/` prefix: keep the tail after the last `/`. No-op when the
/// name has no `/`. e.g. `openai/gpt-5.4` → `gpt-5.4`.
pub(crate) fn strip_provider_prefix(name: &str) -> &str {
    match name.rfind('/') {
        Some(pos) => &name[pos + 1..],
        None => name,
    }
}

/// Strip a `[...]` bracketed suffix such as the `[1m]` context-window tag.
/// Returns the part before the first `[`, trailing whitespace trimmed. No-op
/// when the name has no `[`. e.g. `glm-5.2[1m]` → `glm-5.2`.
pub(crate) fn strip_brackets(name: &str) -> &str {
    match name.find('[') {
        Some(pos) => name[..pos].trim_end(),
        None => name,
    }
}

/// Strip a trailing ISO date `-YYYY-MM-DD` (11 chars). No-op when absent.
pub(crate) fn strip_iso_date_suffix(name: &str) -> &str {
    let bytes = name.as_bytes();
    if bytes.len() > 11 && name.is_char_boundary(bytes.len() - 11) {
        let tail = &name[bytes.len() - 11..];
        if tail.is_ascii()
            && tail.as_bytes()[0] == b'-'
            && tail[1..5].bytes().all(|b| b.is_ascii_digit())
            && tail.as_bytes()[5] == b'-'
            && tail[6..8].bytes().all(|b| b.is_ascii_digit())
            && tail.as_bytes()[8] == b'-'
            && tail[9..11].bytes().all(|b| b.is_ascii_digit())
        {
            return &name[..bytes.len() - 11];
        }
    }
    name
}

/// Strip a trailing compact date `-YYYYMMDD` (a `-` followed by exactly 8
/// digits, 9 chars total). No-op when absent.
pub(crate) fn strip_compact_date_suffix(name: &str) -> &str {
    let bytes = name.as_bytes();
    if bytes.len() >= 9 && name.is_char_boundary(bytes.len() - 9) {
        let tail = &name[bytes.len() - 9..];
        if tail.starts_with('-') && tail[1..].bytes().all(|b| b.is_ascii_digit()) {
            return &name[..bytes.len() - 9];
        }
    }
    name
}

/// Canonical model-key normalization, applied at every pricing-match site
/// (parsers, the pricing book's keys and lookup candidates, and the ingest
/// rebill key): ASCII-lowercase, strip a `provider/` prefix, strip `[...]`
/// brackets, then strip trailing ISO (`-YYYY-MM-DD`) and compact
/// (`-YYYYMMDD`) date suffixes. Every sub-step is a no-op when its pattern is
/// absent, so this never changes a name that did not carry that pattern.
pub(crate) fn normalize_model_key(model: &str) -> String {
    let lower = model.to_ascii_lowercase();
    let after_prefix = strip_provider_prefix(&lower);
    let after_brackets = strip_brackets(after_prefix);
    let after_iso = strip_iso_date_suffix(after_brackets);
    strip_compact_date_suffix(after_iso).to_string()
}

#[cfg(test)]
mod tests {
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
        };
        let s: SessionSystemData =
            serde_json::from_str(&serde_json::to_string(&sys).unwrap()).unwrap();
        assert_eq!(s, sys);
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
}

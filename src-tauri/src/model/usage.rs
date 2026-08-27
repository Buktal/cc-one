//! Usage types: per-call / per-turn records, the DTOs crossing the Rust→JS
//! boundary, and the pricing entry. Token counts are `u32` across the boundary;
//! cost is `rust_decimal::Decimal` internally (TEXT in SQLite / JSONL) and `f64`
//! in DTOs (display-only).

use std::str::FromStr;

use rust_decimal::Decimal;

// ---- Token / tool sub-structures (shared by internal record + DTOs) ----

/// Token four-pack (per-call). `u32` across the boundary.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
pub struct TokenCounts {
    pub input: u32,
    pub output: u32,
    pub cache_creation: u32,
    pub cache_read: u32,
}

impl TokenCounts {
    /// Sum of all four buckets — "真实消耗 Tokens" in the dashboard.
    pub fn total(self) -> u32 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_creation)
            .saturating_add(self.cache_read)
    }

    /// "Nothing billable" — every bucket is zero. The single emit gate for the
    /// source parsers (`claude` / `codex` / `gemini` / `grok`): a record
    /// carrying no billable token in any bucket is never emitted. Cache reads
    /// ARE billable, so a cache-read-only row survives this predicate; judged
    /// on the FINAL four-pack (post normalization/clamping) — never on an
    /// intermediate delta shape.
    pub fn is_zero(self) -> bool {
        self.total() == 0
    }

    /// Cache-hit rate as a ratio in [0,1] for display (0 when nothing cacheable).
    /// Denominator = fresh input + cache creation + cache reads — the full
    /// "could have been cached" pool. Matches CC-Switch's cache_hit_rate.
    pub fn cache_hit_rate(self) -> f64 {
        let denom = self.input as f64 + self.cache_creation as f64 + self.cache_read as f64;
        if denom <= 0.0 {
            0.0
        } else {
            self.cache_read as f64 / denom
        }
    }
}

/// Server-side tool usage reported by Claude Code's usage block.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
pub struct ServerToolUse {
    pub web_search: u32,
    pub web_fetch: u32,
}

// ---- Decimal <-> string serde (JSONL stores cost as precision-safe TEXT) ----

/// Serialize `Decimal` as a string (precision-safe for JSONL / SQLite TEXT).
pub fn ser_decimal<S: serde::Serializer>(d: &Decimal, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&d.to_string())
}

/// Deserialize `Decimal` from a string (JSONL reader).
pub fn de_decimal<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Decimal, D::Error> {
    let s = <String as serde::Deserialize>::deserialize(d)?;
    Decimal::from_str(&s).map_err(serde::de::Error::custom)
}

/// Cost split by token bucket, in USD. Computed at ingest, then frozen.
///
/// Internal-only (Decimal precision); DTOs below expose `f64` to the frontend.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CostBreakdown {
    #[serde(serialize_with = "ser_decimal", deserialize_with = "de_decimal")]
    pub input_usd: Decimal,
    #[serde(serialize_with = "ser_decimal", deserialize_with = "de_decimal")]
    pub output_usd: Decimal,
    #[serde(serialize_with = "ser_decimal", deserialize_with = "de_decimal")]
    pub cache_read_usd: Decimal,
    #[serde(serialize_with = "ser_decimal", deserialize_with = "de_decimal")]
    pub cache_creation_usd: Decimal,
    #[serde(serialize_with = "ser_decimal", deserialize_with = "de_decimal")]
    pub total_usd: Decimal,
}

impl CostBreakdown {
    /// Build a breakdown from the four bucket costs; `total` = their sum.
    pub fn from_buckets(
        input: Decimal,
        output: Decimal,
        cache_read: Decimal,
        cache_creation: Decimal,
    ) -> Self {
        let total = input + output + cache_read + cache_creation;
        Self {
            input_usd: input,
            output_usd: output,
            cache_read_usd: cache_read,
            cache_creation_usd: cache_creation,
            total_usd: total,
        }
    }

    /// Decimal total as `f64` for test assertions.
    #[cfg(test)]
    pub fn total_f64(self) -> f64 {
        use rust_decimal::prelude::ToPrimitive;
        self.total_usd.to_f64().unwrap_or(0.0)
    }
}

// ---- Per-call Usage Record (parser output → SQLite + JSONL) ----

/// One model API call (per-call granularity). This is the unit a parser
/// emits, the Local Store stores, and one JSONL line serializes.
///
/// `uuid` is the dedup key. `pricing_model` records the normalized model key
/// used to look up the price, so zero-cost rows can be rebilled precisely
/// (freeze + top-up zero-cost only).
///
/// `turn_duration` is intentionally NOT here — a turn spans multiple calls, so
/// it lives in the separate per-turn [`TurnDuration`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UsageRecord {
    pub uuid: String,
    /// ISO8601 UTC, e.g. `2026-07-13T16:55:22.467Z`.
    pub timestamp: String,
    /// Derived `yyyy-mm-dd` (UTC) for daily bucketing.
    pub day: String,
    /// Billed / mapped model, e.g. `glm-5.2`.
    pub model: String,
    /// Normalized model key used for pricing lookup (rebill key).
    pub pricing_model: String,
    /// Source tag, e.g. `claude_code`.
    pub source: String,
    /// Session this call belongs to: the source log's session identifier
    /// (Claude = the jsonl file stem). Attached grouping info only — the dedup
    /// key stays `(uuid, device_id)`; `session_id` is NOT part of it. Empty when
    /// a parser has not been wired for sessions yet (every source but Claude
    /// in this phase).
    #[serde(default)]
    pub session_id: String,
    /// Owning device's 12-hex id.
    pub device_id: String,
    pub tokens: TokenCounts,
    pub server_tool_use: ServerToolUse,
    /// How the assistant turn terminated: `tool_use` / `end_turn` / ...
    /// Semantic termination reason (NOT an HTTP status). Per-call.
    pub stop_reason: String,
    /// Service tier label, e.g. `standard`. Per-call.
    pub service_tier: String,
    /// Reasoning/thinking iteration count (source array length). 0 when the
    /// model/version records no iterations.
    pub iterations: u32,
    pub cost: CostBreakdown,
}

impl UsageRecord {
    /// Derive the `yyyy-mm-dd` day bucket from an ISO8601 timestamp (UTC).
    /// Falls back to the first 10 chars if parsing fails, so bad input never
    /// drops a record — it just lands in a best-effort bucket.
    pub fn day_from_timestamp(ts: &str) -> String {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(ts) {
            return t.with_timezone(&chrono::Utc).format("%Y-%m-%d").to_string();
        }
        ts.get(..10).unwrap_or("0000-00-00").to_string()
    }
}

// ---- Per-turn TurnDuration (separate grain from per-call records) ----

/// One turn's wall-clock duration. Sourced from the `system/turn_duration`
/// event's `durationMs`. Kept separate from per-call [`UsageRecord`] because a
/// turn spans multiple API calls — the duration is a turn-level fact, not a
/// per-call one.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TurnDuration {
    /// Dedup key (the source `system/turn_duration` event's uuid).
    pub uuid: String,
    pub timestamp: String,
    /// Derived `yyyy-mm-dd` (UTC).
    pub day: String,
    /// Session this turn belongs to (same grouping key as
    /// [`UsageRecord::session_id`]): lets the per-turn aggregates resolve a
    /// project through the sessions table, exactly like usage rows. Rows
    /// collected before this field existed (and old `turns-*.jsonl` artifact
    /// lines, which lack it) deserialize to `""` — no session, so they bucket
    /// into the unknown project.
    #[serde(default)]
    pub session_id: String,
    /// Owning device's 12-hex id.
    pub device_id: String,
    /// Turn wall-clock in milliseconds.
    pub duration_ms: u32,
}

// ---- DTOs crossing the boundary (specta-typed, f64 cost) ----

/// Per-call cost split by token bucket, f64 mirror of [`CostBreakdown`] for
/// the frontend. The table shows the total; the expandable row detail shows
/// the buckets so "why is this call expensive" is answerable.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct LogCostBreakdown {
    pub input_usd: f64,
    pub output_usd: f64,
    pub cache_read_usd: f64,
    pub cache_creation_usd: f64,
}

/// One row of the request-log table. Beyond the visible columns, carries the
/// full per-call fields the row-detail panel shows (cost buckets, session,
/// tier, iterations, tool use) — one query, zero extra round-trip on expand.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct UsageLogRow {
    pub uuid: String,
    pub timestamp: String,
    pub model: String,
    /// Normalized model key used for pricing lookup (rebill key).
    pub pricing_model: String,
    pub source: String,
    /// Session this call belongs to (Claude = the jsonl file stem). Empty when
    /// the parser has not been wired for sessions yet.
    pub session_id: String,
    pub device_id: String,
    pub tokens: TokenCounts,
    pub stop_reason: String,
    /// Service tier label, e.g. `standard`. Empty when unrecorded.
    pub service_tier: String,
    /// Reasoning/thinking iteration count. 0 when unrecorded.
    pub iterations: u32,
    pub server_tool_use: ServerToolUse,
    pub total_cost_usd: f64,
    pub cost: LogCostBreakdown,
}

/// Aggregate totals over a filtered range.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct UsageStats {
    pub request_count: u32,
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_read_tokens: u32,
    /// Cache-hit ratio in [0,1].
    pub cache_hit_rate: f64,
    pub total_cost_usd: f64,
    /// Aggregate over TurnDuration rows in range (per-turn grain).
    pub turn_count: u32,
    pub avg_turn_duration_ms: f64,
    /// 95th-percentile turn duration (ms) over the same turn rows — smallest
    /// duration whose cumulative share reaches 95%. `None` when no turn rows
    /// are in range.
    pub p95_turn_duration_ms: Option<f64>,
    /// Turn-duration histogram over the same turn rows: counts into
    /// `[<10s, 10–30s, 30–60s, >60s]` (the dashboard's four duration bands).
    pub turn_duration_buckets: [u32; 4],
}

/// Per-model aggregate row (for breakdown tables / model filter).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ModelStatsRow {
    pub model: String,
    pub request_count: u32,
    pub total_tokens: u32,
    pub total_cost_usd: f64,
    /// Cache-hit rate over this model's cacheable input pool, [0,1]
    /// (`TokenCounts::cache_hit_rate`, computed at query time).
    pub cache_hit_rate: f64,
}

/// One project bucket at USAGE grain (the dashboard's project dimension,
/// #106): `usage_records` grouped by the owning session's `project_identity`,
/// so the bucket sums equal `UsageStats`'s totals under the same filter
/// exactly — time bounds run on usage timestamps, the hero's caliber. (The
/// sessions page's `ProjectStatsRow` instead selects sessions by
/// `last_active_at` — a sessions-grain caliber answering "where sessions
/// ran"; this one answers "where usage landed".) Every usage row lands in
/// exactly one bucket: rows with a session map by the identity rule, while
/// session-less rows AND rows whose session carries no launch dir both fall
/// to the synthetic [`UNKNOWN_PROJECT`](crate::model::UNKNOWN_PROJECT) bucket
/// — attribution missing either way. Note the project FILTER's sentinel stays
/// the stricter NOT-EXISTS form, so picking the unknown bucket narrows to its
/// session-less share only.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ProjectUsageRow {
    /// Bucket key: a project identity, or the unknown sentinel.
    pub project: String,
    /// Whether this is the unknown bucket — rides as DATA (like
    /// `ProjectCandidates::unknown`) so the frontend never pattern-matches
    /// the sentinel literal.
    pub is_unknown: bool,
    /// Distinct `(session_id, device_id)` pairs among the bucket's rows that
    /// own a session row (0 for the sentinel's session-less share by
    /// definition).
    pub session_count: u32,
    pub request_count: u32,
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_read_tokens: u32,
    /// Cache-hit ratio over the bucket's cacheable pool, [0,1]
    /// (`TokenCounts::cache_hit_rate`).
    pub cache_hit_rate: f64,
    pub total_cost_usd: f64,
    /// `MAX(usage timestamp)` in the bucket — recency for display.
    pub last_active_at: String,
}

/// One session bucket at usage grain (#106 dashboard session section):
/// `usage_records` grouped by `(session_id, device_id)`, INNER-joined to the
/// sessions table — only sessions that EXIST in the store (本机采集 ∪ 拉回的
/// 远程收藏快照) appear; session-less usage surfaces only in the project
/// dimension's unknown bucket. Display fields (title / agent_type /
/// started_at) ride along from the session row. `turn_count` merges the
/// per-session turn rows — the turn grain ignores the model / source facets
/// (no such column; same caliber note as `UsageStats`'s turn aggregate).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionUsageRow {
    pub session_id: String,
    pub device_id: String,
    /// Display title: `custom_title` when set, else `title_orig`.
    pub title: String,
    /// `""` = main session; non-empty = subagent type tag.
    pub agent_type: String,
    /// Session start (ISO8601); always present (the inner join guarantees a
    /// session row).
    pub started_at: String,
    /// `MAX(usage timestamp)` in the window — the bucket's recency.
    pub last_active_at: String,
    /// Turns recorded for this session under the filter's applicable facets.
    pub turn_count: u32,
    pub request_count: u32,
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_read_tokens: u32,
    pub total_cost_usd: f64,
}

/// One device bucket at usage grain (the dashboard's device dimension,
/// #107): `usage_records` grouped by `device_id`, in the exact shape of
/// `query_models` — pure usage aggregates, every `UsageFilter` facet applies
/// (project included, through the one WHERE builder), and the bucket sums
/// equal `UsageStats`'s totals under the same filter exactly. Registry facts
/// (display name, which device is "this machine") are NOT joined here — the
/// frontend merges `list_devices` for them, the same division as the device
/// dropdown. `last_active_at` is the device's newest usage timestamp in the
/// window: for this machine its latest activity, for a peer the latest usage
/// that reached this store (arriving with the last pull — the recency the
/// card shows as 最近同步).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct DeviceUsageRow {
    /// Owning device's 12-hex id (the grouping key).
    pub device_id: String,
    pub request_count: u32,
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_read_tokens: u32,
    /// Cache-hit ratio over the bucket's cacheable pool, [0,1]
    /// (`TokenCounts::cache_hit_rate`).
    pub cache_hit_rate: f64,
    pub total_cost_usd: f64,
    /// `MAX(usage timestamp)` in the bucket — recency for display.
    pub last_active_at: String,
}

/// One point on the trend chart. `day` carries the bucket key: a `YYYY-MM-DD`
/// UTC day (`TrendBucket::Day`) or a `YYYY-MM-DDTHH` local hour
/// (`TrendBucket::Hour`). The field keeps the `day` name for wire stability.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct TrendPoint {
    pub day: String,
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_read_tokens: u32,
    pub total_cost_usd: f64,
    /// Usage rows in this bucket — feeds the daily-request bar chart (the
    /// token sums alone can't answer "how many calls").
    pub request_count: u32,
}

/// Trend aggregation granularity. `Day` groups on the UTC `day` column
/// (cross-device deterministic); `Hour` groups on local-time hour,
/// used for the single-day zoom where per-day resolution collapses to one bar.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, specta::Type)]
pub enum TrendBucket {
    Day,
    Hour,
}

/// Filter args shared by stats / trend / logs queries.
///
/// All fields optional; `None` means "no constraint". `device_scope` is the
/// semantic cache-key axis: `None` = all devices.
///
/// Range bounds are ISO8601 **timestamps**, not `day` strings. The `day` column
/// is a UTC whole-day bucket (cross-device determinism), so a local
/// "today" in a non-UTC zone (e.g. UTC+8) straddles two UTC days; filtering on
/// `day` would drop early-morning rows. The frontend converts its local-day
/// range to UTC timestamps, and we filter on `timestamp` (amendment:
/// `day` stays the UTC bucket for grouping/trend only).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct UsageFilter {
    /// Inclusive lower ISO8601 UTC timestamp, e.g. `2026-07-21T16:00:00Z`.
    pub from_ts: Option<String>,
    /// Inclusive upper ISO8601 UTC timestamp.
    pub to_ts: Option<String>,
    pub model: Option<String>,
    pub source: Option<String>,
    pub device_scope: Option<String>,
    /// Scope to one project (identity 口径, same rule as the sessions-side
    /// filter): a row matches when its session's `project_dir` maps to this
    /// project identity via the `project_identity` SQL scalar — so usage from
    /// a Claude Code worktree session counts under the PARENT project. Rows
    /// without a session id belong to no project and never match. The
    /// [`UNKNOWN_PROJECT`](crate::model::UNKNOWN_PROJECT) sentinel inverts the
    /// match to NOT EXISTS a session row — the unknown bucket (remote usage
    /// without a pulled favorite snapshot, session-less legacy rows).
    /// `None`/empty =
    /// no constraint. Also applied to the per-turn aggregates in `UsageStats`
    /// (`turn_durations` carries `session_id`).
    pub project: Option<String>,
}

/// Query params for the request-log endpoint (adds paging to `UsageFilter`).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct LogsQuery {
    pub filter: UsageFilter,
    pub limit: u32,
    pub offset: u32,
}

// ---- Pricing ----

/// A pricing entry: USD per 1M tokens for each bucket.
///
/// Cost crosses as `f64` for the UI; internally stored as Decimal TEXT.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct PricingEntry {
    /// Normalized model key (primary key).
    pub model_key: String,
    pub display_name: String,
    /// USD per 1M input tokens.
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,
    pub cache_creation_per_million: f64,
    /// True when seeded from LiteLLM upstream, false when user-defined/edited.
    pub is_builtin: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table-driven pin of the parsers' single emit gate
    /// ([`TokenCounts::is_zero`]): a record is dropped iff ALL FOUR buckets are
    /// zero — cache_read alone is billable and keeps the row alive, and any
    /// single non-zero bucket does too.
    #[test]
    fn is_zero_is_true_exactly_when_all_four_buckets_are_zero() {
        let cases: &[(&str, TokenCounts, bool)] = &[
            ("all zero (default)", TokenCounts::default(), true),
            (
                "explicit zeros",
                TokenCounts {
                    input: 0,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
                true,
            ),
            (
                "only input",
                TokenCounts {
                    input: 1,
                    ..Default::default()
                },
                false,
            ),
            (
                "only output",
                TokenCounts {
                    output: 1,
                    ..Default::default()
                },
                false,
            ),
            (
                "only cache_creation",
                TokenCounts {
                    cache_creation: 1,
                    ..Default::default()
                },
                false,
            ),
            (
                "only cache_read (billable — row survives)",
                TokenCounts {
                    cache_read: 5000,
                    ..Default::default()
                },
                false,
            ),
            (
                "every bucket set",
                TokenCounts {
                    input: 10,
                    output: 20,
                    cache_creation: 30,
                    cache_read: 40,
                },
                false,
            ),
        ];
        for (name, tokens, want) in cases {
            assert_eq!(tokens.is_zero(), *want, "{name}");
        }
    }
}

//! Shared timestamp helpers.

use std::sync::atomic::{AtomicU64, Ordering};

/// Last emitted millisecond epoch, kept monotonic across calls.
static LAST_MS: AtomicU64 = AtomicU64::new(0);

/// The timestamp shapes the source parsers actually meet. Which variant a
/// given source field carries is source knowledge, decided by the parser —
/// the entry never guesses a unit.
pub(crate) enum SourceTimestamp<'a> {
    /// Epoch milliseconds (opencode's ms-integer SQLite columns).
    Millis(i64),
    /// Epoch seconds (grok's integer timestamps below its ms threshold).
    Secs(i64),
    /// An RFC3339 string in any UTC offset (grok's string timestamps).
    Rfc3339(&'a str),
}

/// 唯一入口：任意源时间戳 → canonical 形式（[`canonical_iso`]）。落库前必经
/// 此处：`usage_records.timestamp` 是 TEXT 列，范围筛选与跨源 `ORDER BY` 靠
/// 字典序，同列只允许一种 ISO 拼写——`+00:00` 偏移后缀或非毫秒精度混进来，
/// 边界比较就退化为运气。
///
/// `None` = 值存在但解释不成历法时间（越界整数 / 非 RFC3339 字符串）。入口
/// 不伪造时刻，回退策略归调用方：用量行走 `fallback_timestamp` 的采集时刻
/// 回填，会话原文 ts 按「宁缺勿假」留空。
pub(crate) fn source_timestamp_to_iso(ts: SourceTimestamp<'_>) -> Option<String> {
    let dt = match ts {
        SourceTimestamp::Millis(ms) => chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms),
        SourceTimestamp::Secs(secs) => chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0),
        SourceTimestamp::Rfc3339(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()?
                .with_timezone(&chrono::Utc),
        ),
    };
    dt.map(canonical_iso)
}

/// The one canonical spelling every stored timestamp carries:
/// `2026-07-28T01:57:02.123Z` — UTC, `Z`-suffixed, millisecond precision.
/// Kept as a single point so "Z 结尾、毫秒精度" is a property of one function,
/// not a convention each call site re-promises.
fn canonical_iso(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// ISO8601 UTC "now" with millisecond precision (e.g. `2026-07-28T01:57:02.123Z`).
/// Used as a last-resort timestamp when a source omits one, and as the
/// written-at marker for DB rows.
///
/// Monotonic: consecutive calls within the same millisecond (or after a
/// backward clock jump) return strictly increasing timestamps. Row
/// `updated_at` values feed latest-wins merge reads — a tie would fall to
/// "first seen" and a same-ms overwrite could silently lose; the invariant
/// lives here, not in callers.
pub(crate) fn now_iso() -> String {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let ms = LAST_MS
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            let last = last as i64;
            Some(if now_ms > last {
                now_ms as u64
            } else {
                last as u64 + 1
            })
        })
        .unwrap_or(now_ms as u64);
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .map(canonical_iso)
        .unwrap_or_default()
}

/// Format epoch **milliseconds** as ISO8601 UTC with millisecond precision
/// (e.g. `2026-07-20T13:26:10.000Z`), matching `now_iso`'s format so
/// source-derived timestamps sort alongside "now" fallbacks. OpenCode's
/// `session` / `message` tables store `time_created` / `time_updated` as
/// ms-epoch integers. Falls back to `now_iso` for out-of-range inputs so a bad
/// source timestamp never breaks ordering (the None case of
/// [`source_timestamp_to_iso`], resolved here instead of at its callers).
pub(crate) fn epoch_millis_to_iso(ms: i64) -> String {
    source_timestamp_to_iso(SourceTimestamp::Millis(ms)).unwrap_or_else(now_iso)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The monotonic invariant behind latest-wins merge reads: two consecutive
    /// `now_iso()` calls are strictly increasing even inside the same
    /// millisecond, so a same-ms overwrite can never tie with an earlier row.
    #[test]
    fn now_iso_is_strictly_monotonic() {
        let a = now_iso();
        let b = now_iso();
        assert!(a < b, "same-ms calls must not tie ({a} == {b})");
    }

    /// The invariant the store leans on, stated at its single enforcement
    /// point: every shape the parsers hand over comes out as ONE spelling —
    /// UTC, `Z`-suffixed, millisecond precision — so the shared TEXT column
    /// never mixes layouts (the exact equality against the `.000Z` literal is
    /// the pin). Especially the `+00:00`-offset spelling that bare
    /// `to_rfc3339()` emits must normalize to `Z`.
    #[test]
    fn source_timestamp_to_iso_yields_the_single_canonical_form() {
        let canonical = "2023-11-14T22:13:20.000Z";
        assert_eq!(
            source_timestamp_to_iso(SourceTimestamp::Millis(1_700_000_000_000)).as_deref(),
            Some(canonical)
        );
        assert_eq!(
            source_timestamp_to_iso(SourceTimestamp::Secs(1_700_000_000)).as_deref(),
            Some(canonical)
        );
        assert_eq!(
            source_timestamp_to_iso(SourceTimestamp::Rfc3339("2023-11-14T22:13:20Z")).as_deref(),
            Some(canonical)
        );
        assert_eq!(
            source_timestamp_to_iso(SourceTimestamp::Rfc3339("2023-11-14T22:13:20.000+00:00"))
                .as_deref(),
            Some(canonical)
        );
        assert_eq!(
            source_timestamp_to_iso(SourceTimestamp::Rfc3339("2023-11-14T23:13:20+01:00"))
                .as_deref(),
            Some(canonical)
        );
    }

    /// `None` only for values that exist but mean no calendar time — the entry
    /// never fabricates one; each caller applies its own fallback (usage rows
    /// backfill to collection time, transcript ts stays empty).
    #[test]
    fn source_timestamp_to_iso_rejects_uninterpretable_values() {
        assert_eq!(
            source_timestamp_to_iso(SourceTimestamp::Millis(i64::MAX)),
            None
        );
        assert_eq!(
            source_timestamp_to_iso(SourceTimestamp::Secs(i64::MAX)),
            None
        );
        assert_eq!(
            source_timestamp_to_iso(SourceTimestamp::Rfc3339("not-a-time")),
            None
        );
    }
}

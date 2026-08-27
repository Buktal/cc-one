//! Shared per-line JSONL fold skeleton (架构审查Ⅳ候选⑧).
//!
//! Every line-cursor parser used to hand-write the same six steps — enumerate
//! lines, derive the 1-based line number, trim, skip blank lines, parse with
//! serde, account a malformed line as skipped — and the invariant that matters
//! most lived only in those hand copies: **a malformed line is counted as
//! skipped exactly once, by the pass whose cursor first crosses it** (a
//! re-collect reads the whole head again and must not recount it — otherwise
//! `CollectResult::lines_skipped` inflates on every no-op refresh). This
//! module owns that skeleton once; the per-parser fold bodies shrink to what
//! only they can know: how to route one parsed line, and what session
//! metadata to feed it. Event-type dispatch and [`super::SessionMetaAcc`]
//! feeding deliberately stay in the callers — the walker yields
//! `(line_no, parsed)` and does not absorb per-source routing.
//!
//! Exactly two policies (one per parser shape — a third would mean some
//! parser's real semantics were being flattened to fit, not modeled):
//!   - [`LineFoldPolicy::ObserveAllCountPastCursor`] — claude/codex shape.
//!     Those parsers rebuild refreshable full-file state on every pass
//!     (session metadata through `SessionMetaAcc`; codex additionally its
//!     cumulative token baseline), so pre-cursor lines are parsed and yielded
//!     too and the caller gates emission on the yielded line number.
//!   - [`LineFoldPolicy::GateFirst`] — grok shape. Grok's line files carry no
//!     full-file state to rebuild (its session meta lives in the sibling
//!     `summary.json`, a separate mtime-only file), so the cursor gate
//!     precedes everything and pre-cursor lines are neither parsed nor
//!     yielded.
//!
//! The cheap pre-serde candidate gate (codex's marker substring filter) is a
//! predicate parameter here rather than a layer: a line rejecting it is noise
//! — never parsed, never counted (see the `lines_skipped` declaration in
//! `super`).

use serde::de::DeserializeOwned;

/// Which side of the incremental cursor the walker lets lines through, and
/// how a malformed line's skip is accounted. See the module docs for which
/// parser shape each variant models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineFoldPolicy {
    /// claude/codex shape: every non-blank candidate line is parsed and
    /// yielded — including lines at or before `start_line` (the refreshable
    /// full-file state must be rebuilt from them); the caller compares the
    /// yielded `line_no` against `start_line` to gate emission. A serde
    /// failure counts as skipped only when the line is PAST the cursor —
    /// already-counted head lines are not recounted on a re-collect.
    ObserveAllCountPastCursor,
    /// grok shape: the cursor gate precedes everything — lines at or before
    /// `start_line` are never trimmed, parsed, or yielded — so a serde
    /// failure (necessarily past the cursor) counts unconditionally.
    GateFirst,
}

/// Fold `text`'s lines through the shared skeleton and return the skipped
/// count. Owns, in order: 1-based line numbering (matching the stored
/// cursor), the [`LineFoldPolicy`]'s cursor gate, trimming, blank-line
/// skipping, the `is_candidate` pre-serde gate, serde parsing into `T`, and
/// skipped accounting per the policy. `on_line` receives `(line_no, parsed)`
/// for every line the policy lets through — all parsed lines under
/// [`LineFoldPolicy::ObserveAllCountPastCursor`] (the caller gates emission
/// on `line_no`), only past-cursor ones under [`LineFoldPolicy::GateFirst`].
pub(super) fn for_accepted_lines<T>(
    text: &str,
    start_line: i64,
    policy: LineFoldPolicy,
    is_candidate: impl Fn(&str) -> bool,
    mut on_line: impl FnMut(i64, T),
) -> u32
where
    T: DeserializeOwned,
{
    let mut skipped = 0u32;
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx as i64 + 1; // 1-based, matching the cursor
        if policy == LineFoldPolicy::GateFirst && line_no <= start_line {
            continue;
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // Pre-serde cheap gate (codex's marker substring filter): not a
        // candidate ⇒ noise, neither parsed nor counted.
        if !is_candidate(line) {
            continue;
        }
        let value = match serde_json::from_str::<T>(line) {
            Ok(v) => v,
            Err(_) => {
                // The invariant, per policy: GateFirst only ever sees
                // past-cursor lines (its gate ran first), so every failure
                // counts; ObserveAll re-reads the counted head every pass and
                // therefore counts only the incremental tail.
                if policy == LineFoldPolicy::GateFirst || line_no > start_line {
                    skipped += 1;
                }
                continue;
            }
        };
        on_line(line_no, value);
    }
    skipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// Run the walker over `lines` (joined with newlines), collecting the
    /// yielded `(line_no, raw parsed json)` pairs plus the skipped count.
    fn walk(
        text: &str,
        start_line: i64,
        policy: LineFoldPolicy,
        is_candidate: impl Fn(&str) -> bool,
    ) -> (Vec<(i64, Value)>, u32) {
        let mut out = Vec::new();
        let skipped = for_accepted_lines(
            text,
            start_line,
            policy,
            is_candidate,
            |line_no, v: Value| {
                out.push((line_no, v));
            },
        );
        (out, skipped)
    }

    fn line(json: &str) -> String {
        json.to_string()
    }

    // ---- numbering / trim / blank lines ----

    /// 1-based numbering (matching the stored cursor), and blank / whitespace
    /// lines neither yield nor count — but they still occupy a line number.
    #[test]
    fn numbers_lines_from_one_and_skips_blanks_without_counting() {
        let text = [
            line(r#"{"a":1}"#),
            line(""),
            line("   "),
            line(r#"{"a":2}"#),
        ]
        .join("\n");
        let (got, skipped) = walk(&text, 0, LineFoldPolicy::GateFirst, |_| true);
        assert_eq!(skipped, 0, "blank lines are noise, not parse failures");
        let nos: Vec<i64> = got.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            nos,
            vec![1, 4],
            "1-based numbering; blank lines keep their slot"
        );
        assert_eq!(got[1].1.get("a").and_then(|v| v.as_i64()), Some(2));
    }

    // ---- the two policies' skipped windows ----

    /// ObserveAllCountPastCursor: pre-cursor lines are YIELDED (the meta /
    /// baseline window) but a malformed HEAD line is not recounted — only the
    /// incremental tail counts.
    #[test]
    fn observe_all_yields_precursor_lines_but_counts_malformed_tail_only() {
        let text = [
            line(r#"{"a":1}"#), // pre-cursor, valid → yielded
            line("{not json"),  // pre-cursor, malformed → NOT recounted
            line(r#"{"a":2}"#), // past cursor, valid → yielded
            line("{also bad"),  // past cursor, malformed → counted
        ]
        .join("\n");
        let (got, skipped) = walk(&text, 2, LineFoldPolicy::ObserveAllCountPastCursor, |_| {
            true
        });
        assert_eq!(
            got.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            vec![1, 3],
            "pre-cursor parsed lines are yielded too (full-file meta window)"
        );
        assert_eq!(skipped, 1, "only the past-cursor malformed line counts");
    }

    /// GateFirst: the cursor gate precedes everything — pre-cursor lines are
    /// neither yielded nor counted, and a past-cursor malformed line counts
    /// unconditionally.
    #[test]
    fn gate_first_yields_nothing_precursor_and_counts_past_cursor_failures() {
        let text = [
            line(r#"{"a":1}"#), // pre-cursor, valid → NOT yielded
            line("{not json"),  // pre-cursor, malformed → not counted
            line(r#"{"a":2}"#), // past cursor, valid → yielded
            line("{also bad"),  // past cursor, malformed → counted
        ]
        .join("\n");
        let (got, skipped) = walk(&text, 2, LineFoldPolicy::GateFirst, |_| true);
        assert_eq!(
            got.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            vec![3],
            "pre-cursor lines never reach the callback"
        );
        assert_eq!(skipped, 1);
        // Full scan (start_line 0) degenerates to parse-everything for both
        // policies — same count, same yields.
        for policy in [
            LineFoldPolicy::GateFirst,
            LineFoldPolicy::ObserveAllCountPastCursor,
        ] {
            let (got, skipped) = walk(&text, 0, policy, |_| true);
            assert_eq!(got.len(), 2);
            assert_eq!(skipped, 2);
        }
    }

    // ---- UTF-8 half-line (the lossy-read tail) ----

    /// A line flushed mid-write carries U+FFFD from the lossy decode and its
    /// JSON structure is broken — it is one malformed line like any other and
    /// counts ONCE, while every complete line before it parses.
    #[test]
    fn utf8_half_line_lands_as_fffd_and_counts_once() {
        // "中" = E4 B8 AD, only E4 B8 landed → U+FFFD inside a broken string
        // (the unterminated string makes the line one plain JSON failure).
        let text = [line(r#"{"a":1}"#), line("{\"a\": \"\u{FFFD}")].join("\n");
        let (got, skipped) = walk(&text, 0, LineFoldPolicy::GateFirst, |_| true);
        assert_eq!(got.len(), 1, "the complete line parses");
        assert_eq!(skipped, 1, "the U+FFFD tail is one counted failure");
    }

    // ---- the pre-serde candidate gate ----

    /// A non-candidate line is dropped before serde: not yielded, not
    /// counted — even when its JSON is malformed (codex's marker gate is a
    /// perf design that pins the `lines_skipped` counting rule).
    #[test]
    fn candidate_gate_rejects_before_parse_silently() {
        let text = [
            line(r#"{"type":"noise","payload":{}}"#), // fails the gate, valid json
            line("{garbage"),                         // fails the gate, malformed
            line(r#"{"type":"keep","payload":{}}"#),  // passes the gate
        ]
        .join("\n");
        let (got, skipped) = walk(&text, 0, LineFoldPolicy::ObserveAllCountPastCursor, |l| {
            l.contains("\"keep\"")
        });
        assert_eq!(got.len(), 1, "only gate-passing lines are parsed");
        assert_eq!(skipped, 0, "gate-rejected lines are never counted");
    }
}

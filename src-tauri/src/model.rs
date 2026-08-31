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
//!   - [`project`] — the project dimension: unknown-bucket sentinel, dropdown
//!     candidates, and the `project_identity` rule.
//!   - [`session`] — session, transcript, and snapshot types.
//!   - [`usage`] — per-call / per-turn usage records, DTOs, and pricing.
//!
//! The model-key normalizer lives in this file (single source of truth).

mod device;
mod project;
mod provider;
mod session;
mod usage;

pub use device::*;
pub use project::*;
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
mod tests;
